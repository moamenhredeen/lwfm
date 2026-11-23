use embed_manifest::manifest::{DpiAwareness, MaxVersionTested, Setting};
use embed_manifest::{embed_manifest, new_manifest};

fn main() {
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        let manifest = new_manifest("lwfm")
            .dpi_awareness(DpiAwareness::PerMonitorV2)
            .long_path_aware(Setting::Enabled)
            .max_version_tested(MaxVersionTested::Windows11);
        embed_manifest(manifest).expect("unable to embed manifest");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
