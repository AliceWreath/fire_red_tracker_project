# Third-Party Licenses

This project (fire\_red\_tracker and fire\_red\_aggregator binaries, plus all supporting library crates
in this workspace) is distributed under the **MIT License** — see [LICENSE](LICENSE).

The binaries statically link or bundle a number of third-party crates from crates.io. Their licenses
are listed below, grouped by SPDX expression. All licenses are compatible with MIT distribution.

To regenerate this listing from the current `Cargo.lock`:

```
cargo install cargo-license
cargo-license
```

---

## Special License Notices

### MPL-2.0 — `colored 2.x`

`colored` (terminal colour output) is licensed under the **Mozilla Public License 2.0**.
MPL-2.0 is a *file-level* weak-copyleft license: if you distribute a modified version of
`colored`'s own source files you must make those modifications available under MPL-2.0.
Using `colored` as a dependency in a binary does not affect the license of the surrounding code.

### Apache-2.0 OR GPL-2.0 — `self_cell 1.x`

`self_cell` is dual-licensed. We elect the **Apache-2.0** option; GPL-2.0 does not apply.

### Apache-2.0 AND ISC — `ring 0.17`

`ring` carries both the Apache-2.0 and ISC licenses simultaneously. Both are permissive;
attribution requirements are met by including this file.

### (Apache-2.0 OR ISC) AND ISC — `aws-lc-rs 1.x`
### Complex multi-license — `aws-lc-sys 0.x`

`aws-lc-rs` and `aws-lc-sys` bundle the AWS-LC C library, whose components carry a mixture of
Apache-2.0, ISC, MIT, MIT-0, and BSD-3-Clause terms. All are permissive.

### CDLA-Permissive-2.0 — `webpki-root-certs`, `webpki-roots`

These crates contain the Mozilla CA certificate bundle, distributed under the
**Community Data License Agreement – Permissive, Version 2.0**. CDLA-Permissive-2.0 permits
use, modification, and redistribution with no copyleft requirements.

### OFL-1.1 AND Ubuntu-font-1.0 — `epaint_default_fonts`

`epaint_default_fonts` (part of egui) bundles the Hack and Ubuntu font families. The
**SIL Open Font License 1.1** and **Ubuntu Font License 1.0** permit embedding fonts in
applications; selling the fonts as a standalone product is prohibited.

### Apache-2.0 OR LGPL-2.1-or-later OR MIT — `r-efi`

`r-efi` is tri-licensed. We elect **MIT**; LGPL does not apply.

### (Apache-2.0 OR MIT) AND NCSA — `libfuzzer-sys`

`libfuzzer-sys` links against LLVM's libFuzzer, which carries the **NCSA license** (a
BSD-style, 3-clause permissive license).

### (Apache-2.0 OR MIT) AND Unicode-3.0 — `unicode-ident`

`unicode-ident` includes Unicode data tables licensed under the
**Unicode License v3** (formerly Unicode-DFS-2016), a permissive data license.

---

## Dependency Listing by License

### MIT

adler, android-properties, animated-image, approx, ashpd, atk-sys,
av-data, av1-grain, bindgen, block2, bzip2, bzip2-sys, cairo-sys-rs,
cairo-macros, cesu8, color_quant, combine, console, crunchy, dav1d,
dav1d-sys, derive_builder, derive_builder_core, dispatch, dlib, dtoa,
endi, enumset, enumset_derive, fax, fs_extra, gdk-pixbuf-sys, gio-sys,
glib, glib-macros, glib-sys, glob, gobject-sys, gtk-sys, gtk3-macros,
hound, image-webp, indicatif, interpolate_name, jni, jni-macros,
jni-sys, jni-sys-macros, jni-sys-macros, libffi, libffi-sys, libgsf-sys,
librespot-core, librespot-discovery, librespot-metadata, librespot-protocol,
libtiff-sys, lofty, loop9, matchers, md-5, memoffset, mime, mp4, nalu_types,
new_debug_unreachable, noop_proc_macro, no_std_io2, nu-ansi-term,
ob_system, ob_types, ob_types_macro, objc2, objc2-app-kit,
objc2-cloud-kit, objc2-contacts, objc2-core-data, objc2-core-image,
objc2-core-location, objc2-encode, objc2-foundation, objc2-link-presentation,
objc2-metal, objc2-quartz-core, objc2-symbols, objc2-ui-kit,
objc2-uniform-type-identifiers, objc2-user-notifications, objc-sys,
once_cell_polyfill, opus, opus-sys, orbclient, ouvrt, pango-sys,
pastey, plain, proc-macro-crate, profiling, profiling-procmacros,
quick-error, rgb, ringbuf, rustc_version, scoped-tls, sha1,
sharded-slab, simd_cesu8, simd_helpers, siphasher, smol_str,
stable_deref_trait, static_assertions, stringprep, strsim,
ureq-proto, utf8_iter, utf8-zero, valuable, version_check, videodecoder,
wayland-client, wayland-backend, wayland-csd-frame, wayland-cursor,
wayland-protocols, wayland-protocols-experimental, wayland-protocols-misc,
wayland-protocols-plasma, wayland-protocols-wlr, wayland-scanner,
wayland-sys, webbrowser, weezl, x11-dl, x11rb, x11rb-protocol,
xattr, xcursor, xkbcommon-dl, xml-rs, y4m, zmij

### Apache-2.0 OR MIT (largest group — ~367 crates)

Includes: accesskit, ahash, anyhow, arc-swap, arrayvec, axum, axum-core, base64,
bincode, bytes, cc, cfg-if, chacha20, clap, concurrent-queue, cookie, crc32fast,
crossbeam-deque, crossbeam-utils, der, digest, eframe, egui, epaint, emath,
event-listener, fastrand, flate2, fnv, futures-\*, getrandom, half, hashbrown,
heck, hex, hmac, http, httparse, hyper, hyper-util, idna, image, indexmap,
itoa, lazy\_static, libc, lock\_api, log, memmap2, mio, naga, nix, nom,
num-traits, objc2-\*, once\_cell, openssl-probe, parking\_lot, paste, peniko,
percent-encoding, phf, pin-project, pin-project-lite, pkcs8, png, polling,
postgres, postgres-protocol, postgres-types, ppv-lite86, proc-macro2,
quinn, quinn-proto, quote, rand, rand\_chacha, rand\_core, rayon, read-fonts,
regex, regex-automata, reqwest, rfd, rustls-pki-types, rustls-platform-verifier,
serde, serde\_json, serde\_derive, sha2, signature, slab, smallvec,
smithay-client-toolkit, socket2, spki, syn, synstructure, tar, tempfile,
thiserror, thread\_local, time, tokio, tokio-macros, tokio-postgres,
tokio-rustls, tokio-tungstenite, tokio-util, toml, tower, tower-http,
tracing, tracing-attributes, tracing-core, tracing-log, tracing-subscriber,
ttf-parser, tungstenite, typenum, unicode-bidi, unicode-normalization,
unicode-width, url, ureq, uuid, vello\_common, vello\_cpu, want,
wasm-bindgen, wasm-bindgen-futures, wasm-bindgen-macro, web-sys, wgpu,
wgpu-core, wgpu-hal, wgpu-types, windows-sys, winnow, zbus, zeroize,
zvariant, and many more.

### Apache-2.0

ab_glyph, ab_glyph_rasterizer, accesskit\_winit, codespan-reporting,
gethostname, gl\_generator, glutin, glutin\_egl\_sys, glutin\_glx\_sys,
glutin\_wgl\_sys, khronos\_api, owned\_ttf\_parser, self-replace, spirv,
sync\_wrapper, winit

### Apache-2.0 AND ISC

ring

### Apache-2.0 AND MIT

dpi

### (Apache-2.0 OR ISC) AND ISC

aws-lc-rs

### (Apache-2.0 OR ISC OR MIT) AND … AND BSD-3-Clause AND ISC AND MIT

aws-lc-sys

### Apache-2.0 OR Apache-2.0 WITH LLVM-exception OR MIT

linux-raw-sys, rustix, wasi, wasip2, wasip3, wasm-encoder, wasm-metadata,
wasmparser, wit-bindgen, wit-bindgen-core, wit-bindgen-rust,
wit-bindgen-rust-macro, wit-component, wit-parser, zipsign-api

### Apache-2.0 OR BSD-1-Clause OR MIT

fiat-crypto

### Apache-2.0 OR BSD-2-Clause OR MIT

zerocopy, zerocopy-derive

### Apache-2.0 OR BSD-3-Clause

moxcms, pxfm

### Apache-2.0 OR BSD-3-Clause OR MIT

num\_enum, num\_enum\_derive

### Apache-2.0 OR BSL-1.0

ryu

### Apache-2.0 OR BSL-1.0 OR MIT

wasite, whoami

### Apache-2.0 OR CC0-1.0

imgref

### Apache-2.0 OR CC0-1.0 OR MIT-0

dunce

### Apache-2.0 OR GPL-2.0

self\_cell *(Apache-2.0 elected)*

### Apache-2.0 OR ISC OR MIT

hyper-rustls, rustls, rustls-native-certs

### Apache-2.0 OR LGPL-2.1-or-later OR MIT

r-efi *(MIT elected)*

### (Apache-2.0 OR MIT) AND NCSA

libfuzzer-sys

### (Apache-2.0 OR MIT) AND OFL-1.1 AND Ubuntu-font-1.0

epaint\_default\_fonts

### (Apache-2.0 OR MIT) AND Unicode-3.0

unicode-ident

### (Apache-2.0 OR MIT) AND BSD-3-Clause

matchit

### 0BSD OR Apache-2.0 OR MIT

adler2

### BSD-2-Clause

arrayref, instant, miniz\_oxide, num-bigint-dig, stable\_deref\_trait (some versions)

### BSD-3-Clause

bitflags (some versions), cranelift-entity, ordered-float, pixeli,
subtle, thread\_id

### BSD-3-Clause AND MIT

matchit (some configurations)

### BSL-1.0

atomic, lmdb-rkv, ryu (some configurations)

### CC0-1.0

constant\_time\_eq

### CDLA-Permissive-2.0

webpki-root-certs, webpki-roots

### ISC

rustls-webpki, untrusted, void

### MIT OR Unlicense

byteorder, byteorder-lite, either (some versions), memchr, same-file

### MPL-2.0

colored *(see Special License Notices above)*

### Unicode-3.0

icu\_\* crates (unicode normalisation data)

### Zlib

miniz\_oxide (some versions), tinyvec, tinyvec\_macros

---

*Last updated 2026-06-09. Generated with `cargo-license 0.6` against `Cargo.lock`.*
