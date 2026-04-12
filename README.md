# Open Wages

**A clean-room, open-source Rust reimplementation of the *Wages of War: The Business of Battle* (1996) engine.**

*Wages of War* was a turn-based tactical squad game developed by Random Games and published by New World Computing / 3DO. All three companies are now defunct. This project builds a modern engine that plays the original game using your own copy of the original data files — no copyrighted material is redistributed.

Inspired by [OpenXCOM](https://openxcom.org/), which did the same for *X-COM: UFO Defense*.

## Status

🚧 **Alpha — Playable!** The full game loop works: hire mercs from the original 57-merc roster, accept contracts, deploy on real isometric mission maps, move and shoot in tactical combat, complete missions, get paid, take the next contract. Built from scratch in Rust with SDL2. Visuals are placeholder (colored squares for units, correct terrain tiles), but the gameplay loop is functional end-to-end.

## What This Is

- A **new engine** written from scratch in Rust
- Reads the **original game's data files** (you supply your own copy)
- Aims for **feature parity** with the original game, then improvements
- **Moddable** — the data layer is plaintext .dat files, which the original game already supported editing

## What This Is NOT

- Not a copy of the original source code (clean-room implementation)
- Not a distribution of the original game (you need your own copy)
- Not affiliated with Random Games, New World Computing, or 3DO

## Key Features (Planned)

- **Initiative-based combat** — all units act in initiative order, not player-then-enemy
- **Suppression system** — incoming fire affects morale and action points
- **Weather effects** — rain, fog, night, sandstorm affect gameplay
- **Mercenary management** — hiring, training, equipment, reputation
- **Isometric rendering** — faithful diamond-projection tile engine
- **Modern QoL** — resizable window, save anywhere, mod support

## Building

```bash
# Prerequisites: Rust toolchain, SDL2 development libraries
cargo build --workspace
```

## Running

```bash
# Point to your original game data directory
RUST_LOG=info cargo run -p ow-app -- --data-dir /path/to/wages-of-war/

# Verbose logging
RUST_LOG=debug cargo run -p ow-app -- --data-dir /path/to/wages-of-war/
```

## Project Structure

| Crate | Purpose |
|-------|---------|
| `ow-data` | Parsers for original game files (.dat, sprites, maps) |
| `ow-core` | Game rules, combat, economy, AI — no rendering dependencies |
| `ow-render` | Isometric tile/sprite renderer |
| `ow-audio` | Sound and music playback |
| `ow-app` | Main executable, event loop, window management |

## Contributing

See `CLAUDE.md` for development guidelines. This project uses `cargo fmt` and `cargo clippy` for code quality.

## Legal

This is a **clean-room reimplementation**. No original code, assets, or copyrighted material is included in this repository.

- **No original code was referenced.** All engine behavior is derived from black-box observation of the running game and its data files.
- **No original assets are redistributed.** You must supply your own legally obtained copy of *Wages of War: The Business of Battle* to play.
- **Not affiliated with the original creators.** This project has no connection to Random Games, New World Computing, or 3DO. All three companies are defunct.
- **No monetization.** This project is and will always be completely free. No donations, no sponsorships, no paid features. This is a community preservation effort, not a commercial product.
- **Interoperability rights.** Reverse engineering for interoperability is protected under DMCA §1201(f) and EU Software Directive Art. 6.

## License

Dual-licensed under your choice of:

- [MIT License](LICENSE-MIT)
- [Apache License 2.0](LICENSE-APACHE)

---

---

---

## Support This Project

If you find this project useful, consider buying me a coffee! Your support helps me keep building and sharing open-source tools.

[![Donate via PayPal](https://img.shields.io/badge/Donate-PayPal-blue.svg?logo=paypal)](https://www.paypal.me/baal_hosting)

**PayPal:** [baal_hosting@live.com](https://paypal.me/baal_hosting)

Every donation, no matter how small, is greatly appreciated and motivates continued development. Thank you!
