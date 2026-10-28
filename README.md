# Chimera 🤖

**Local-first AI agent engine** – a modular Rust workspace that ships a DAG task scheduler, a unified LLM client (OpenAI, Anthropic, Google, Mistral, Cohere, Ollama), sandboxed tool execution (shell, filesystem), and a Tauri desktop UI with dark charcoal + electric-orange glassmorphism.

---

## ✨ Features

| Crate | Description |
|------|-------------|
| `chimera-core-engine` | Async DAG scheduler with topological ordering, concurrency control, cycle detection, failure propagation, metrics. |
| `chimera-llm-client` | Provider-agnostic `LLMClient` trait, streaming (SSE) + non-streaming completions, embeddings, tool calling, retries. |
| `chimera-tools` | `ShellExecutor` & `FileExecutor` with path/command allow-lists, resource limits, per‑call timeouts. |
| `chimera-tauri-app` | Frameless Tauri 2 window, custom titlebar, IPC commands for every subsystem, realtime LLM token streaming. |

---

## 🚀 Quick Start

```bash
# 1. Install Rust (MSVC toolchain on Windows)
# 2. Clone & build
git clone https://github.com/<your-org>/chimera.git
cd chimera
cargo build --release --workspace

# 3. Run the desktop app
cargo run --release -p chimera-tauri-app
```

> **Requirements**: Rust ≥1.78, Node ≥20 (for Tauri dev), WebView2 (Windows), WebKitGTK (Linux), Xcode 15+ (macOS).

---

## 📦 Crate Overview

```
chimera/
├─ Cargo.toml                 # workspace root
├─ crates/
│  ├─ core-engine/            # DAG scheduler
│  ├─ llm-client/             # LLM abstraction + providers
│  ├─ tools/                  # Sandbox shell / file ops
│  └─ tauri-app/              # Desktop UI (Tauri 2)
└─ README.md
```

Each crate publishes its own `Cargo.toml` with `workspace = true` dependencies.

---

## 🛠 Development

```bash
# Run tests for all crates
cargo test --workspace

# Lint
cargo clippy --workspace -- -D warnings

# Format
cargo fmt --all
```

### Adding a new LLM provider
1. Implement `LLMClient` in `llm-client/src/providers/your_provider.rs`.
2. Re‑export from `llm-client/src/lib.rs`.
3. Add the provider enum variant in `types.rs::Provider`.

### Extending tooling
Implement the `Tool` trait (see `tools/src/types.rs`) and register it in `ToolRegistry`.

---

## 📄 License

MIT © 2024 Chimera Contributors  
See [LICENSE](LICENSE) for full text.

---

## 💖 Support the Project

[![Saweria](https://img.shields.io/badge/Donate-Saweria-FF6B00?logo=saweria&logoColor=white)](https://saweria.co/roketdesign15)
[![Buy Me A Coffee](https://img.shields.io/badge/Buy%20Me%20A%20Coffee-FFKHolis89A-FF6B00?logo=buymeacoffee&logoColor=white)](https://buymeacoffee.com/ffkholis89a)
[![GitHub Sponsors](https://img.shields.io/badge/Sponsor-GitHub-%23EA4AAA?logo=githubsponsors&logoColor=white)](https://github.com/sponsors/<your-github-username>)

---

## 🏷️ Badges

![Rust](https://img.shields.io/badge/rust-1.78%2B-orange?logo=rust)
![Tauri](https://img.shields.io/badge/tauri-2.0-24C8DB?logo=tauri)
![License](https://img.shields.io/badge/license-MIT-blue)
![Build](https://img.shields.io/github/actions/workflow/status/<your-org>/chimera/ci.yml?branch=main&logo=githubactions)
![Crates.io](https://img.shields.io/crates/v/chimera-core-engine?label=core-engine&logo=rust)
![Crates.io](https://img.shields.io/crates/v/chimera-llm-client?label=llm-client&logo=rust)
![Crates.io](https://img.shields.io/crates/v/chimera-tools?label=tools&logo=rust)

---

## 🙌 Contributing

1. Fork → create a feature branch.  
2. Run `cargo fmt && cargo clippy && cargo test`.  
3. Open a PR with a clear description.

All contributions are welcome – issues, docs, benchmarks, new providers, UI polish, etc.

---

## 📚 Resources

- **Architecture docs** – `docs/ARCHITECTURE.md` (to be added)  
- **API reference** – `cargo doc --open --workspace`  
- **Changelog** – `CHANGELOG.md` (auto‑generated via `git-cliff`)

---

> **Chimera** – *compose, schedule, execute, repeat.* 🚀