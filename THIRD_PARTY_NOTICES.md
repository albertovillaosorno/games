# Third-party notices

## Current scope

The engine uses SHA-2 for save integrity. The shared asset crate uses
SHA-2 and Serde JSON for canonical manifests. The importer uses ZIP
and the pure-Rust zlib-rs Deflate backend. The native platform
adapter uses Rustls, Ring and SHA-2 for authenticated LAN transport and
Nix for descriptor-relative archive staging. Linux graphics and audio use
Rust-SDL2 with the system SDL2 library (Zlib licence), linked dynamically.
Optional packaged application entry uses Inventory registration and the
Android JNI bindings. The Android development packaging adapter builds
SDL2 from the locked Rust-SDL2 source, bundles its shared library and
marks the minimal SDL Java entry adaptation. No
original game assets, original game code or candidate project source is
incorporated or distributed by this repository. The development validator
uses Serde JSON, TOML and SHA-2; platform tests use Rcgen.

## Planned reuse

The sole designated upstream reuse candidate is Craft Libraries
(`storytold/craft-libs`): <https://github.com/storytold/craft-libs>.

Its NOTICE identifies the ArtCraft Team and craft-libs contributors as
copyright holders and offers eligible upstream code under MIT or
Apache-2.0. This is not a claim of current incorporation or endorsement.

Verify the actual version, applicable licence, copyright and NOTICE
requirements before adopting any material. Where upstream offers an
Apache-2.0 option, prefer that option when appropriate. Preserve required
notices and mark modifications when distributing adapted source.

## Resolved dependencies

The following licence expressions come from cached Cargo package metadata
for the current lockfile. They are not a completed distribution audit.

| Package | Declared licence |
| --- | --- |
| `asn1-rs-derive` | MIT OR Apache-2.0 |
| `asn1-rs-impl` | MIT/Apache-2.0 |
| `asn1-rs` | MIT OR Apache-2.0 |
| `autocfg` | Apache-2.0 OR MIT |
| `bit-vec` | Apache-2.0 OR MIT |
| `bitflags` | MIT OR Apache-2.0 |
| `block-buffer` | MIT OR Apache-2.0 |
| `bumpalo` | MIT OR Apache-2.0 |
| `bytes` | MIT |
| `cc` | MIT OR Apache-2.0 |
| `cfg-if` | MIT OR Apache-2.0 |
| `cfg_aliases` | MIT |
| `combine` | MIT |
| `const-oid` | Apache-2.0 OR MIT |
| `constant_time_eq` | CC0-1.0 OR MIT-0 OR Apache-2.0 |
| `cpufeatures` | MIT OR Apache-2.0 |
| `crc32fast` | MIT OR Apache-2.0 |
| `crypto-common` | MIT OR Apache-2.0 |
| `data-encoding` | MIT |
| `der-parser` | MIT OR Apache-2.0 |
| `deranged` | MIT OR Apache-2.0 |
| `digest` | MIT OR Apache-2.0 |
| `displaydoc` | MIT OR Apache-2.0 |
| `equivalent` | Apache-2.0 OR MIT |
| `find-msvc-tools` | MIT OR Apache-2.0 |
| `flate2` | MIT OR Apache-2.0 |
| `getrandom` | MIT OR Apache-2.0 |
| `hashbrown` | MIT OR Apache-2.0 |
| `hybrid-array` | MIT OR Apache-2.0 |
| `indexmap` | Apache-2.0 OR MIT |
| `inventory` | MIT OR Apache-2.0 |
| `itoa` | MIT OR Apache-2.0 |
| `jni` | MIT OR Apache-2.0 |
| `jni-macros` | MIT OR Apache-2.0 |
| `jni-sys` | MIT OR Apache-2.0 |
| `jni-sys-macros` | MIT OR Apache-2.0 |
| `js-sys` | MIT OR Apache-2.0 |
| `lazy_static` | MIT OR Apache-2.0 |
| `libc` | MIT OR Apache-2.0 |
| `log` | MIT OR Apache-2.0 |
| `memchr` | Unlicense OR MIT |
| `minimal-lexical` | MIT/Apache-2.0 |
| `nix` | MIT |
| `nom` | MIT |
| `num-bigint` | MIT OR Apache-2.0 |
| `num-conv` | MIT OR Apache-2.0 |
| `num-integer` | MIT OR Apache-2.0 |
| `num-traits` | MIT OR Apache-2.0 |
| `oid-registry` | MIT OR Apache-2.0 |
| `once_cell` | MIT OR Apache-2.0 |
| `pkg-config` | MIT OR Apache-2.0 |
| `powerfmt` | MIT OR Apache-2.0 |
| `proc-macro2` | MIT OR Apache-2.0 |
| `quote` | MIT OR Apache-2.0 |
| `r-efi` | MIT OR Apache-2.0 OR LGPL-2.1-or-later |
| `rcgen` | MIT OR Apache-2.0 |
| `ring` | Apache-2.0 AND ISC |
| `rustc_version` | MIT OR Apache-2.0 |
| `rusticata-macros` | MIT/Apache-2.0 |
| `rustls-pki-types` | MIT OR Apache-2.0 |
| `rustls-webpki` | ISC |
| `rustls` | Apache-2.0 OR ISC OR MIT |
| `rustversion` | MIT OR Apache-2.0 |
| `same-file` | Unlicense/MIT |
| `sdl2` | MIT |
| `sdl2-sys` | MIT AND Zlib |
| `semver` | MIT OR Apache-2.0 |
| `serde_core` | MIT OR Apache-2.0 |
| `serde_derive` | MIT OR Apache-2.0 |
| `serde_json` | MIT OR Apache-2.0 |
| `serde_spanned` | MIT OR Apache-2.0 |
| `serde` | MIT OR Apache-2.0 |
| `sha2` | MIT OR Apache-2.0 |
| `shlex` | MIT OR Apache-2.0 |
| `simd_cesu8` | Apache-2.0 OR MIT |
| `simdutf8` | MIT OR Apache-2.0 |
| `subtle` | BSD-3-Clause |
| `syn` | MIT OR Apache-2.0 |
| `synstructure` | MIT |
| `thiserror-impl` | MIT OR Apache-2.0 |
| `thiserror` | MIT OR Apache-2.0 |
| `time-core` | MIT OR Apache-2.0 |
| `time-macros` | MIT OR Apache-2.0 |
| `time` | MIT OR Apache-2.0 |
| `toml_datetime` | MIT OR Apache-2.0 |
| `toml_parser` | MIT OR Apache-2.0 |
| `toml_writer` | MIT OR Apache-2.0 |
| `toml` | MIT OR Apache-2.0 |
| `typed-path` | MIT OR Apache-2.0 |
| `typenum` | MIT OR Apache-2.0 |
| `unicode-ident` | (MIT OR Apache-2.0) AND Unicode-3.0 |
| `untrusted` | ISC |
| `version-compare` | MIT |
| `walkdir` | Unlicense/MIT |
| `wasi` | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| `wasm-bindgen-macro-support` | MIT OR Apache-2.0 |
| `wasm-bindgen-macro` | MIT OR Apache-2.0 |
| `wasm-bindgen-shared` | MIT OR Apache-2.0 |
| `wasm-bindgen` | MIT OR Apache-2.0 |
| `web-sys` | MIT OR Apache-2.0 |
| `winapi-util` | Unlicense OR MIT |
| `windows-link` | MIT OR Apache-2.0 |
| `windows-sys` | MIT OR Apache-2.0 |
| `windows-targets` | MIT OR Apache-2.0 |
| `windows_aarch64_gnullvm` | MIT OR Apache-2.0 |
| `windows_aarch64_msvc` | MIT OR Apache-2.0 |
| `windows_i686_gnu` | MIT OR Apache-2.0 |
| `windows_i686_gnullvm` | MIT OR Apache-2.0 |
| `windows_i686_msvc` | MIT OR Apache-2.0 |
| `windows_x86_64_gnu` | MIT OR Apache-2.0 |
| `windows_x86_64_gnullvm` | MIT OR Apache-2.0 |
| `windows_x86_64_msvc` | MIT OR Apache-2.0 |
| `winnow` | MIT |
| `x509-parser` | MIT OR Apache-2.0 |
| `yasna` | MIT OR Apache-2.0 |
| `zeroize_derive` | Apache-2.0 OR MIT |
| `zeroize` | Apache-2.0 OR MIT |
| `zip` | MIT |
| `zlib-rs` | Zlib |
| `zmij` | MIT |

Refresh this inventory from Cargo.lock after dependency changes. Before
distribution, retain the required upstream licence texts, copyright and
attribution notices for all included dependencies. This notice does not
replace the applicable third-party licence terms.

Original game names identify compatibility targets only; their trademarks
and proprietary materials remain the property of their respective owners.

## Android JNI licence texts

The published `jni` and `jni-macros` 0.22.4 crates omit their root
licence files. The identical upstream MIT text is preserved in
`tools/build/licenses/jni-rs-0.22.4/LICENSE-MIT`, from revisions
`5ae9458a4ec44c5318f37ddc7569c1d4ae8a69e7` and
`33045a124105c939d1e2cbdcb5a39e5d868ffa03`, respectively.
The `jni-sys-macros` 0.4.1 MIT text at revision
`64d77b7a5f119d7b55b4e2c169a4668067ff59e6` is byte-identical
to the cached `jni-sys` 0.4.1 licence. Packaging uses these exact
versioned texts instead of silently omitting licence documents.
