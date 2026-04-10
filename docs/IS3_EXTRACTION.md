# InstallShield v3 Cabinet Extraction

## Problem
The Wages of War ISO contains an InstallShield v3 installer. The game's executable (`Wow.exe`) and other files are compressed inside `SETUP/_SETUP.1` using PKWARE DCL Implode compression. The 16-bit installer (`SETUP.EXE`) cannot run on 64-bit Windows.

## Solution
Use the `idecomp` Python tool to extract all files from the IS3 cabinet.

### Prerequisites
```bash
git clone https://github.com/lephilousophe/idecomp.git
```

### Extract all game files
```bash
mkdir -p data/extracted
python3 idecomp/idecomp.py -C data/extracted data/SETUP/_SETUP.1
```

This extracts 622 files (129MB) organized into groups:
- **Group1**: `Wow.exe` (the game executable, 1MB PE32), font files (`.CHR`), cursors
- **Group2-9**: Game data files (sprites, maps, missions, etc.)
- **Group10**: WinG DLLs and `WINGPAL.IMS` (master palette)
- **Group11**: Voice/sound files (`.VLA`, `.VLS`, `.WAV`)
- **Group12-13**: Help files, icons

### Key files for RE
- `Group1/Wow.exe` — Main game executable (PE32, i386, 6 sections, ~1MB)
- `Group10/WINGPAL.IMS` — Master VGA palette (embedded in a DLL stub)
- `Group1/ICFONT*.CHR` — Bitmap fonts used by the game
- `Group10/WING*.DLL` — WinG rendering libraries (the game uses WinG, not DirectDraw)

### Technical notes
- IS3 signature: `0x8C655D13` at offset 0 of both `.1` and `.LIB` files
- Compression: PKWARE DCL Implode (blast algorithm), each file independently compressed
- The `_SETUP.LIB` file contains headers, `_SETUP.1` contains the compressed data
- The `unshield` tool does NOT support IS3 — only IS5+
- The game references `wow.pal` for palette loading (see strings in Wow.exe)
