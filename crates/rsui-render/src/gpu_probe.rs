//! Explicit probe entry points. Normal draw/render never creates or waits for queries.
use super::*;
use std::time::{Duration, Instant};

impl Renderer {
    /// Copy the actual current window target BEFORE Present, only for the separate
    /// correctness lane. No redraw, independent App, or command export is involved.
    pub fn current_window_hash_for_diagnostics(&self) -> Result<u64, RenderError> {
        let Target::Window(chain) = &self.target else {
            return Ok(0);
        };
        let (width, height) = self.size;
        // SAFETY: live swap chain; staging description matches its BGRA target.
        // Every read is bounded by RowPitch/height, and Unmap follows the copy.
        unsafe {
            let back: ID3D11Texture2D = chain.GetBuffer(0)?;
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            back.GetDesc(&mut desc);
            desc.Usage = D3D11_USAGE_STAGING;
            desc.BindFlags = 0;
            desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
            desc.MiscFlags = 0;
            let mut staging = None;
            self.device
                .CreateTexture2D(&desc, None, Some(&mut staging))?;
            let staging = staging.expect("created staging texture");
            self.context.CopyResource(&staging, &back);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
            let mut hash = 14695981039346656037u64;
            for row in 0..height as usize {
                let bytes = std::slice::from_raw_parts(
                    mapped
                        .pData
                        .cast::<u8>()
                        .add(row * mapped.RowPitch as usize),
                    width as usize * 4,
                );
                for b in bytes {
                    hash = (hash ^ u64::from(*b)).wrapping_mul(1099511628211);
                }
            }
            self.context.Unmap(&staging, 0);
            Ok(hash)
        }
    }

    /// The actual adapter used by this renderer, for reproducible probe records.
    pub fn adapter_for_diagnostics(&self) -> Result<String, RenderError> {
        let dxgi: IDXGIDevice = self.device.cast()?;
        // SAFETY: only reads the adapter description from a valid device.
        let desc = unsafe { dxgi.GetAdapter()?.GetDesc()? };
        let end = desc
            .Description
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(desc.Description.len());
        Ok(String::from_utf16_lossy(&desc.Description[..end]))
    }

    /// Hardware-only offscreen lane: never silently substitutes WARP.
    pub fn offscreen_hardware_for_diagnostics(size: (u32, u32)) -> Result<Self, RenderError> {
        let (device, context) = create_device(D3D_DRIVER_TYPE_HARDWARE)?;
        let size = (size.0.max(1), size.1.max(1));
        let (target, view) = offscreen_target(&device, size, false)?;
        Self::finish(device, context, target, Some(view), size)
    }

    /// A bounded, opt-in disjoint/timestamp interval around draw, plus CPU draw time.
    /// GPU ticks include any queue idle time between these markers, not just busy cycles.
    /// Query allocation and readback are outside CPU time. No presentation is included.
    pub fn time_draw_for_diagnostics(
        &mut self,
        draw: impl FnOnce(&mut Self) -> Result<(), RenderError>,
    ) -> Result<(Duration, Option<Duration>, &'static str), RenderError> {
        let query = |kind| -> Result<ID3D11Query, RenderError> {
            let mut result = None;
            // SAFETY: valid description and owned output on this renderer's device.
            unsafe {
                self.device.CreateQuery(
                    &D3D11_QUERY_DESC {
                        Query: kind,
                        MiscFlags: 0,
                    },
                    Some(&mut result),
                )?;
            }
            Ok(result.expect("successful query creation"))
        };
        let disjoint = query(D3D11_QUERY_TIMESTAMP_DISJOINT)?;
        let start = query(D3D11_QUERY_TIMESTAMP)?;
        let end = query(D3D11_QUERY_TIMESTAMP)?;
        // SAFETY: queries belong to this context's device and are correctly paired.
        unsafe {
            self.context.Begin(&disjoint);
            self.context.End(&start);
        }
        let time = Instant::now();
        let result = draw(self);
        let cpu = time.elapsed();
        // SAFETY: closes the interval even on a failed draw.
        unsafe {
            self.context.End(&end);
            self.context.End(&disjoint);
            self.context.Flush();
        }
        result?;
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut data = D3D11_QUERY_DATA_TIMESTAMP_DISJOINT::default();
        let mut first = 0u64;
        let mut last = 0u64;
        loop {
            // The generated wrapper maps S_FALSE to Ok(()). Test HRESULT == S_OK
            // directly; frequency/tick sentinels cannot prove query completion.
            let get = |query: &ID3D11Query, output: *mut c_void, size| unsafe {
                // SAFETY: live context/query, output size/type supplied below;
                // Query inherits Asynchronous with the same COM interface prefix.
                query_ready((self.context.vtable().GetData)(
                    self.context.as_raw(),
                    query.as_raw(),
                    output,
                    size,
                    D3D11_ASYNC_GETDATA_DONOTFLUSH.0 as u32,
                ))
            };
            let ready = get(&disjoint, (&raw mut data).cast(), size_of_val(&data) as u32)?
                && get(&start, (&raw mut first).cast(), 8)?
                && get(&end, (&raw mut last).cast(), 8)?;
            if ready {
                let status = if data.Disjoint.as_bool() {
                    "disjoint"
                } else if data.Frequency == 0 {
                    "zero-frequency"
                } else if last < first {
                    "reversed"
                } else if last == first {
                    "valid-zero"
                } else {
                    "valid"
                };
                return Ok((cpu, interval(&data, first, last), status));
            }
            if Instant::now() >= deadline {
                return Ok((cpu, None, "timeout"));
            }
            std::thread::yield_now();
        }
    }
}

// Keep HRESULT readiness independent of output sentinels: S_FALSE may leave
// plausible prior values in an output buffer. Failed queries propagate errors.
fn query_ready(status: HRESULT) -> Result<bool, RenderError> {
    status.ok()?;
    Ok(status == HRESULT(0))
}
fn interval(data: &D3D11_QUERY_DATA_TIMESTAMP_DISJOINT, first: u64, last: u64) -> Option<Duration> {
    (!data.Disjoint.as_bool() && data.Frequency > 0 && last >= first)
        .then(|| Duration::from_secs_f64((last - first) as f64 / data.Frequency as f64))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn readiness_requires_exact_success_and_equal_ticks_are_valid() {
        assert!(!query_ready(HRESULT(1)).unwrap()); // S_FALSE, even with stale nonzero outputs
        assert!(query_ready(HRESULT(0)).unwrap());
        assert!(query_ready(HRESULT(0x80004005u32 as i32)).is_err());
        let mut data = D3D11_QUERY_DATA_TIMESTAMP_DISJOINT {
            Frequency: 1_000_000,
            Disjoint: false.into(),
        };
        assert_eq!(interval(&data, 42, 42), Some(Duration::ZERO));
        assert_eq!(interval(&data, 0, 0), Some(Duration::ZERO));
        assert_eq!(interval(&data, 43, 42), None);
        data.Disjoint = true.into();
        assert_eq!(interval(&data, 42, 44), None);
        data.Disjoint = false.into();
        data.Frequency = 0;
        assert_eq!(interval(&data, 42, 44), None);
    }
}
