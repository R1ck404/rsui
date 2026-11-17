//! Puts `rsui-tsx.manifest` into the rsui-tsx program. It asks for the segment heap as the process heap, which
//! made getImageData+putImageData twice as fast and left the rest as it was. What `rsui-tsx --pack` makes is a copy of this
//! program, so it has it too. Only the program gets it, not tests or examples, and only from Microsoft's linker, which can put
//! one in by itself.

fn main() {
    println!("cargo:rerun-if-changed=rsui-tsx.manifest");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
        return;
    }
    let folder = std::env::var("CARGO_MANIFEST_DIR").expect("cargo says where the package is");
    let manifest = std::path::Path::new(&folder).join("rsui-tsx.manifest");
    println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}", manifest.display());
    // No `requestedExecutionLevel` of its own, which a program without a manifest doesn't have either.
    println!("cargo:rustc-link-arg-bins=/MANIFESTUAC:NO");
}
