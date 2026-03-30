fn main() {
    // For now, just rerun if templates change
    println!("cargo:rerun-if-changed=templates/");
}
