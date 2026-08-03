//! Origin Private File System support for WebKitGTK.
//!
//! WebKitGTK 2.52 exposes the File System API behind feature flags, but its
//! writable stream commit path currently fails with `ENOTSUP` on Linux.  The
//! API therefore passes feature detection and then loses every write at
//! `close()`.  Until that backend is complete, inject a document-start OPFS
//! implementation backed by IndexedDB.  IndexedDB remains origin-isolated,
//! persistent, quota-managed browser storage, so this preserves the privacy
//! and persistence contract applications expect from OPFS.

const OPFS_JS: &str = include_str!("../assets/opfs.js");

fn enabled() -> bool {
    !std::env::var("HWATU_NATIVE_OPFS")
        .is_ok_and(|value| matches!(value.as_str(), "1" | "on" | "true"))
}

/// Install the OPFS implementation before page scripts run.
pub fn wire_view(view: &webkit6::WebView) {
    use webkit6::prelude::*;
    if !enabled() {
        return;
    }
    let Some(manager) = view.user_content_manager() else {
        return;
    };
    manager.add_script(&webkit6::UserScript::new(
        OPFS_JS,
        webkit6::UserContentInjectedFrames::AllFrames,
        webkit6::UserScriptInjectionTime::Start,
        &[],
        &[],
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shim_implements_the_venice_support_probe() {
        for operation in [
            "getDirectory",
            "getFileHandle",
            "createWritable",
            "write(chunk)",
            "close()",
            "removeEntry",
        ] {
            assert!(
                OPFS_JS.contains(operation),
                "missing OPFS operation {operation}"
            );
        }
    }

    #[test]
    fn shim_is_persistent_and_origin_scoped_by_indexeddb() {
        assert!(OPFS_JS.contains("indexedDB.open"));
        assert!(OPFS_JS.contains("dev.hwatu.opfs"));
        assert!(!OPFS_JS.contains("localStorage"));
    }

    #[test]
    fn shim_supports_stream_and_random_access_writes() {
        assert!(OPFS_JS.contains("extends WritableStream"));
        assert!(OPFS_JS.contains("seek(position)"));
        assert!(OPFS_JS.contains("truncate(size)"));
        assert!(OPFS_JS.contains("keepExistingData"));
    }

    #[test]
    fn native_backend_has_an_explicit_escape_hatch() {
        std::env::set_var("HWATU_NATIVE_OPFS", "1");
        assert!(!enabled());
        std::env::set_var("HWATU_NATIVE_OPFS", "0");
        assert!(enabled());
        std::env::remove_var("HWATU_NATIVE_OPFS");
    }
}
