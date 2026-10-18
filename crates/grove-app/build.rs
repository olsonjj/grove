//! Grove's build script: exposes the incrementing build number to the app.

fn main() {
    println!("cargo:rerun-if-changed=build-number");
    let build = std::fs::read_to_string("build-number").expect("read build-number");
    println!("cargo:rustc-env=GROVE_BUILD={}", build.trim());
}
