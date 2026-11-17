//! Compiles the shaders, for the program to ship. Run this after changing `quad.hlsl`.
//!
//! Run with `cargo run -p rsui-render --example compile_shaders`.

use windows::Win32::Graphics::Direct3D::Fxc::{D3DCOMPILE_OPTIMIZATION_LEVEL3, D3DCompile};
use windows::Win32::Graphics::Direct3D::ID3DBlob;
use windows::core::{PCSTR, s};

const SOURCE: &str = include_str!("../src/quad.hlsl");

fn compile(entry: PCSTR, target: PCSTR) -> Vec<u8> {
    let mut code: Option<ID3DBlob> = None;
    let mut errors: Option<ID3DBlob> = None;
    // SAFETY: the source pointer and length describe a valid string, and each output is
    // read only as the blob's own pointer and size say.
    unsafe {
        let result = D3DCompile(
            SOURCE.as_ptr().cast(),
            SOURCE.len(),
            s!("quad.hlsl"),
            None,
            None,
            entry,
            target,
            D3DCOMPILE_OPTIMIZATION_LEVEL3,
            0,
            &mut code,
            Some(&mut errors),
        );
        if let Err(error) = result {
            let details = errors.map_or_else(String::new, |blob| {
                String::from_utf8_lossy(std::slice::from_raw_parts(
                    blob.GetBufferPointer().cast::<u8>(),
                    blob.GetBufferSize(),
                ))
                .into_owned()
            });
            panic!("the shader didn't compile: {error}\n{details}");
        }
        let code = code.expect("a successful compile gives code");
        std::slice::from_raw_parts(code.GetBufferPointer().cast::<u8>(), code.GetBufferSize())
            .to_vec()
    }
}

fn main() {
    let folder = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
    let vertex = compile(s!("vs"), s!("vs_4_0"));
    let pixel = compile(s!("ps"), s!("ps_4_0"));
    let image = compile(s!("image_ps"), s!("ps_4_0"));
    std::fs::write(format!("{folder}/quad_vs.dxbc"), &vertex).expect("can write the shader");
    std::fs::write(format!("{folder}/quad_ps.dxbc"), &pixel).expect("can write the shader");
    std::fs::write(format!("{folder}/image_ps.dxbc"), &image).expect("can write the shader");
    println!("vertex shader: {} bytes", vertex.len());
    println!("pixel shader: {} bytes", pixel.len());
    println!("image shader: {} bytes", image.len());
}
