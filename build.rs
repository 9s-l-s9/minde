fn main() {
    // Locate libguile-3.0 via pkg-config and emit the required link flags.
    pkg_config::Config::new()
        .atleast_version("3.0")
        .probe("guile-3.0")
        .expect("guile-3.0 not found via pkg-config; is it in the guix shell manifest?");
}
