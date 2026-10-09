fn main() {
    println!("cargo:rerun-if-changed=build-number");
    let build = std::fs::read_to_string("build-number").expect("read build-number");
    println!("cargo:rustc-env=GROVE_BUILD={}", build.trim());
}
