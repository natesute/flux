# Onboarding

Quick reference for getting this repo onto your GitHub and continuing development with Claude Code.

## First-time push to GitHub

You'll need: a GitHub account, [`gh` CLI](https://cli.github.com) (recommended) or just `git`, and SSH or HTTPS auth set up for github.com.

### Option A: with the GitHub CLI (easiest)

```bash
cd flux
git init
git add .
git commit -m "Initial commit: v0.2 foundation"
gh repo create natesute/flux --public --source=. --remote=origin --push
```

That's it. The `gh repo create` command makes the empty remote, hooks it up as `origin`, and pushes.

### Option B: plain git

1. Go to https://github.com/new and create an empty repo named `flux` under your `natesute` account. Don't initialize it with a README, license, or .gitignore — we already have those.
2. Then locally:

```bash
cd flux
git init
git add .
git commit -m "Initial commit: v0.2 foundation"
git branch -M main
git remote add origin git@github.com:natesute/flux.git   # or https://github.com/natesute/flux.git
git push -u origin main
```

## After it's on GitHub

Verify it builds before doing anything else:

```bash
cargo build --release
```

If you don't have Rust installed: https://rustup.rs.

You'll also need FFmpeg on your `PATH` for actual rendering (the `render` subcommand pipes frames to it). Test it:

```bash
ffmpeg -version
```

## Working on it with Claude Code

Once it's on GitHub, install Claude Code and from the repo directory just run:

```bash
claude
```

The conventions doc at [`CLAUDE.md`](../CLAUDE.md) is the contract — Claude Code reads it on startup and uses it to know how this codebase is organized, what counts as in-scope, and what requires asking first. The most important parts:

- **Adding a new node** is a ~80-line task: one file in `src/nodes/`, one shader in `shaders/`, register in `src/nodes/mod.rs`, document in `docs/NODE_REFERENCE.md`.
- **Don't add top-level dependencies** without discussing first.
- **Don't change the project file schema** without bumping the version.

The roadmap in [`ROADMAP.md`](ROADMAP.md) tracks what's actually built vs. what's planned. It's the canonical source of truth — when work is finished, the roadmap gets updated to reflect it. Don't claim a feature works unless it appears under "Working" there.

## A reasonable first-session-with-Claude-Code prompt

> Read CLAUDE.md, ARCHITECTURE.md, and ROADMAP.md. Then implement the `chromatic_aberration` node from the v0.2 polish list. Follow the patterns in `bloom.rs` since it's the closest analogue — single input, single output, shader-driven. Update NODE_REFERENCE.md and ROADMAP.md when done.
