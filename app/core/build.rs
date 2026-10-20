fn main() {
    // the web page is embedded into the program: rebuild when it changes
    println!("cargo:rerun-if-changed=../../frontend/dist");
}
