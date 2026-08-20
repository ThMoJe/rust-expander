fn main() {
    // Compile Slint UI files
    slint_build::compile("ui/main.slint").expect("Failed to compile Slint UI");

    // Set Windows subsystem to "windows" to hide the console window
    println!("cargo:rustc-link-arg-bins=/SUBSYSTEM:WINDOWS");
    println!("cargo:rustc-link-arg-bins=/ENTRY:mainCRTStartup");
}
