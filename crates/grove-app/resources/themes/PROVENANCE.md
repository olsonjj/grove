# Theme provenance

Every `*.json` file here except `grove-dark.json` is copied unchanged from
[gpui-kit's `themes/` directory](https://github.com/longbridge/gpui-kit/tree/v0.8.1/themes)
at commit `99576db58ec8103417c41a092e891b4022d24e7e` — the tip of the `v0.8.1`
release tag, matching the gpui-kit version pinned in `Cargo.toml`. One file can
carry several themes (for example `catppuccin.json` = Latte/Frappe/Macchiato/
Mocha), so 20 files register more names than that.

The theme files are part of gpui-kit's code, licensed
[Apache-2.0](https://github.com/longbridge/gpui-kit/blob/v0.8.1/LICENSE-APACHE).

`grove-dark.json` is not vendored: it is Grove's own palette, translated from
the design prototype (see the constants in `crates/grove-app/src/theme.rs` and
the parity tests there). It lives in this directory only so the whole set the
registry learns at startup is one embedded bundle.

To bump a gpui-kit upgrade: copy the new release's `themes/*.json` over these
files, update this note's commit, and keep `grove-dark.json` untouched.
