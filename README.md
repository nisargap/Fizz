# Fizz

Fizz is the physical layer for AI agents. This repository starts with a small Rust API and a static project page, ready to deploy on Vercel.

## Project layout

- `api/status.rs` — native Rust Vercel Function at `/api/status`
- `public/index.html` — static page at `/`
- `Cargo.toml` — Rust package and function binary

## Local development

Install Rust and the [Vercel CLI](https://vercel.com/docs/cli), then run:

```sh
cargo check
vercel dev
```

Open `/` for the project page or `/api/status` for the JSON status response. The function is built by Vercel's Rust runtime; `cargo check` verifies its Rust code locally.

## Deploy

Create or link a Vercel project from this repository with the framework preset **Other** and the repository root as the root directory. Vercel detects `api/status.rs` as a Rust Function and serves `public/index.html` as a static asset. No build command or output directory is needed.

This is a foundation for Fizz; no device control or agent protocol is implemented yet.
