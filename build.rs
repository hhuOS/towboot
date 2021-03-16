//! Our build script.
//!
//! Currently, this just makes certain compile-time environment variables visible to
//! the application.

fn copy_variable(name: &str) {
    // see https://stackoverflow.com/a/51311222/2192464
    println!("cargo:rustc-env={}={}", name, std::env::var(name).unwrap());
}

fn main() {
    // see https://doc.rust-lang.org/cargo/reference/environment-variables.html#environment-variables-cargo-sets-for-build-scripts
    // for the meaning
    copy_variable("TARGET");
    copy_variable("HOST");
    copy_variable("PROFILE");
}
