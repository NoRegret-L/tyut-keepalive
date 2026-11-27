# Temporary upstream patch

Source: https://github.com/marrionesa/tauri-plugin-power-monitor/tree/317e87afcd0832ff9823b22bae4e7ea560eab234
Version: 0.1.0; original crates.io checksum: a32d7afdcf1174326df095c5d3285bf7a116514d06b8cbe65a492fa621051d60.

Only functional change: replace the nonexistent IOKit IOPSRelease declaration and call with CoreFoundation CFRelease on the non-null owned IOPSCopyPowerSourcesInfo result. Existing array RAII and sleep/wake callbacks are untouched. Other platform code is copied unchanged from the initial upstream release commit. Upstream license files are retained. Remove this Cargo patch once an upstream corrected release is available. The published crate archive was not independently byte-compared against this Git commit.
