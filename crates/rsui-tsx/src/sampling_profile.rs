//! Optional native UI-thread sampling. Entirely absent from ordinary builds.
//! Reports contain wall-clock stack samples, including waits, not CPU cycles.

use std::path::PathBuf;
use std::time::Duration;

#[path = "sampler.rs"]
mod sampler;

pub(crate) struct NativeProfile {
    sampler: sampler::Sampler,
    output: PathBuf,
    every: Duration,
    delay: Duration,
}

impl NativeProfile {
    /// Called on the actual run_sized UI thread, after scripts load.
    pub(crate) fn start() -> Option<Self> {
        let output = std::env::var_os("RSUI_SAMPLE").filter(|v| !v.is_empty())?;
        let every = Duration::from_micros(std::env::var("RSUI_PROF_US").ok()
            .and_then(|v| v.parse::<u64>().ok()).unwrap_or(1000).clamp(100, 1_000_000));
        let delay = Duration::from_millis(std::env::var("RSUI_SAMPLE_DELAY_MS").ok()
            .and_then(|v| v.parse::<u64>().ok()).unwrap_or(0).min(86_400_000));
        Some(Self { sampler: sampler::Sampler::start_after(every, delay),
            output: output.into(), every, delay })
    }

    /// Run before this thread returns: its HOST still owns the engine and JIT code.
    pub(crate) fn finish(self) {
        let samples = self.sampler.finish();
        #[cfg(feature = "jit")]
        let jit = crate::bridge::sampling_jit_code();
        let mut names = sampler::Names::new();
        let mut report = format!("Native UI-thread wall-clock stack samples (not CPU cycles).\ninterval_us={} warmup_delay_ms={}\nBuild with release debug=line-tables-only and strip=none for Rust names.\n\n",
            self.every.as_micros(), self.delay.as_millis());
        report.push_str(&sampler::report(&samples, &mut names, 100));
        #[cfg(feature = "jit")]
        report.push_str(&jit_report(&samples, &jit, &mut names));
        if let Err(error) = std::fs::write(&self.output, report) {
            eprintln!("native sampling report {}: {error}", self.output.display());
        }
        // Each sample's stack, outermost first, one a line with `;` between, and how many there were of each, as flame graph
        // tools and a script that wants who called what both read.
        let mut folded: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        for sample in &samples {
            let line: Vec<String> = sample.iter().rev().map(|&pc| names.name(pc).replace(';', ":")).collect();
            *folded.entry(line.join(";")).or_default() += 1;
        }
        let mut text = String::new();
        for (stack, count) in folded {
            text.push_str(&format!("{stack} {count}\n"));
        }
        let mut path = self.output.clone().into_os_string();
        path.push(".folded");
        let _ = std::fs::write(path, text);
    }
}

/// Finishes a profile when it's dropped, for runs that can end at any of several places.
pub(crate) struct FinishOnDrop(Option<NativeProfile>);

impl FinishOnDrop {
    pub(crate) fn new(profile: NativeProfile) -> Self {
        Self(Some(profile))
    }
}

impl Drop for FinishOnDrop {
    fn drop(&mut self) {
        if let Some(profile) = self.0.take() {
            profile.finish();
        }
    }
}

/// With the JIT: which compiled instruction each sample was in, or under (in a helper it called), as compiled code has
/// no unwind information and the sampler's report can only call it `?!?`. By function, then by instruction.
#[cfg(feature = "jit")]
fn jit_report(samples: &[Vec<u64>], code: &[rsui_js::JitCode], names: &mut sampler::Names) -> String {
    use std::collections::HashMap;
    let total = samples.len().max(1) as f64;
    let find = |pc: u64| {
        let pc = pc as usize;
        let function = code.iter().find(|c| (c.start..c.end).contains(&pc))?;
        let at = (pc - function.start) as u32;
        let cold = function.ops.first().is_some_and(|&(_, cold, _)| at >= cold);
        let index = if cold {
            function.ops.iter().rposition(|&(_, c, _)| c <= at)
        } else {
            function.ops.iter().rposition(|&(h, _, _)| h <= at)
        };
        Some((function, index, cold))
    };
    let mut in_code = 0u32;
    let mut functions: HashMap<String, (u32, u32)> = HashMap::new();
    let mut ops: HashMap<String, (u32, u32)> = HashMap::new();
    let mut under: HashMap<String, u32> = HashMap::new();
    for stack in samples {
        let Some((depth, (function, index, cold))) = stack
            .iter()
            .enumerate()
            .find_map(|(depth, &pc)| Some((depth, find(if depth == 0 { pc } else { pc - 1 })?)))
        else {
            continue;
        };
        in_code += 1;
        let own = u32::from(depth == 0);
        let entry = functions.entry(function.name.clone()).or_default();
        entry.0 += own;
        entry.1 += 1;
        let op = match index {
            Some(index) => format!(
                "{} #{index} {}{}",
                function.name,
                function.ops[index].2,
                if cold { " (cold)" } else { "" }
            ),
            None => format!("{} prologue", function.name),
        };
        let entry = ops.entry(op.clone()).or_default();
        entry.0 += own;
        entry.1 += 1;
        if depth > 0 {
            *under.entry(format!("{op}  <-  {}", names.name(stack[0]))).or_default() += 1;
        }
    }
    let sorted = |map: HashMap<String, (u32, u32)>| {
        let mut rows: Vec<(String, (u32, u32))> = map.into_iter().collect();
        rows.sort_by(|a, b| b.1.1.cmp(&a.1.1).then(a.0.cmp(&b.0)));
        rows
    };
    let pct = |n: u32| f64::from(n) * 100.0 / total;
    let mut out = format!(
        "
compiled code: {:.1}% of samples in it or under it

by compiled function (in its code, and in all)
",
        pct(in_code)
    );
    for (name, (own, all)) in sorted(functions).into_iter().take(30) {
        out.push_str(&format!("{:6.1}% {:6.1}%  {name}
", pct(own), pct(all)));
    }
    out.push_str("
by compiled instruction (in its code, and in all)
");
    for (name, (own, all)) in sorted(ops).into_iter().take(80) {
        out.push_str(&format!("{:6.1}% {:6.1}%  {name}
", pct(own), pct(all)));
    }
    out.push_str("
under compiled instructions, by what the sample was in
");
    let mut rows: Vec<(String, u32)> = under.into_iter().collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    for (name, count) in rows.into_iter().take(50) {
        out.push_str(&format!("{:6.1}%  {name}
", pct(count)));
    }
    out
}
