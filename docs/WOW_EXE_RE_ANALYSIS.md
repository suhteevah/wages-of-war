# WOW.EXE Static Reverse Engineering Analysis

**Subject:** `Wow.exe` - Main executable for *Wages of War: The Business of Battle* (1996)
**Developer:** Random Games / New World Computing / 3DO
**Analysis Date:** 2026-04-09
**Methodology:** Clean-room black-box behavioral analysis via static disassembly. No code copied.

## Executive Summary

| Property | Value |
|----------|-------|
| Format | PE32, Intel i386 |
| Size | 1,073,664 bytes (0x106200) |
| Linker | Microsoft Visual C++ 3.0 (MSVC 4.x era) |
| Build Date | Mon Nov 11 14:27:00 1996 |
| Subsystem | Windows GUI |
| Entry Point | 0x4D70E0 (CRT startup, calls WinMain) |
| Image Base | 0x400000 |
| Sections | 6 (.text, .rdata, .data, .idata, .rsrc, .reloc) |

### DLL Dependencies

| DLL | Purpose |
|-----|---------|
| KERNEL32.dll | File I/O (_lopen, _hread, _lclose, _llseek), memory (GlobalAlloc/Lock), INI files (GetPrivateProfileString), process |
| USER32.dll | Window management, message loop, input, display settings, timers, cursors |
| GDI32.dll | Palette management (CreatePalette, SelectPalette, RealizePalette, AnimatePalette, SetSystemPaletteUse) |
| WING32.dll | Fast blitting (WinGCreateDC, WinGCreateBitmap, WinGBitBlt, WinGSetDIBColorTable, WinGRecommendDIBFormat) |
| WINMM.dll | Audio (mciSendCommandA, sndPlaySoundA, waveOutOpen, midiOutSetVolume, timeGetTime) |
| ADVAPI32.dll | Registry (RegOpenKeyExA, RegQueryValueExA, RegCloseKey) |
| comdlg32.dll | File dialogs (GetOpenFileNameA, GetSaveFileNameA) - likely debug/editor mode |

---

## 1. Initialization & Main Loop

### 1.1 CRT Entry Point (0x4D70E0)

The actual PE entry point is the MSVC CRT startup at `0x4D70E0`. This is standard CRT boilerplate:

```
function crt_entry():
    setup SEH frame
    call GetVersion()                           // IAT 0x6935B8
    store version info at 0x4F3E40..0x4F3E4C
    extract major/minor version bytes
    call __cinit()                              // 0x4E08A0 - CRT init
    call _setenvp()                             // 0x4DD140
    call __crtGetEnvironmentStrings()           // 0x4E0880
    cmdline = GetCommandLineA()                 // IAT 0x6935BC
    store at 0x690D78
    call _setargv()                             // 0x4E0130
    if (argc == 0 || cmdline == NULL): exit(-1)
    parse command line (skip quotes, whitespace)
    call GetStartupInfoA()                      // IAT 0x6935C0
    nShowCmd = startupInfo.wShowWindow or SW_SHOWDEFAULT (10)
    call WinMain(hInstance, NULL, cmdline, nShowCmd)   // 0x402027
```

### 1.2 WinMain / Application Entry (called from 0x4D72AE)

WinMain is dispatched through a jump table at `0x402027`. The actual game initialization occurs in the function at approximately `0x4BC370`:

```
function GameInit(hInstance, hPrevInstance):
    timestamp = timeGetTime()                   // IAT 0x693784
    store at 0x680A40, 0x68C3E0

    // Query display capabilities
    hDesktopDC = GetDC(NULL)                    // IAT 0x693720 -> GetDesktopWindow + GetDC
    screenBitsPerPixel = GetDeviceCaps(hDC, BITSPIXEL)   // 0x0C
    screenPlanes = GetDeviceCaps(hDC, PLANES)            // 0x0E
    paletteSupport = GetDeviceCaps(hDC, RASTERCAPS) & RC_PALETTE  // 0x26
    screenWidth = GetDeviceCaps(hDC, HORZRES)            // 0x08 -> stored 0x68B298
    screenHeight = GetDeviceCaps(hDC, VERTRES)           // 0x0A -> stored 0x6815CA
    ReleaseDC()

    // Check if already at 640x480
    if (screenWidth == 640 && screenHeight == 480):
        already640x480 = 1                      // flag at 0x69017E

    // Require 8-bit 256-color mode
    if (bitsPerPixel == 8 && planes == 1):
        // Good - proceed with palette initialization
        call PaletteInit()                      // jumps to 0x4BE08E
    else:
        // Show error: "Wages Of War needs to be in 256 color mode"
        MessageBoxA(0, errorMsg, title, MB_ICONHAND)
        ExitProcess(1)

    // Store module path, strip filename to get app directory
    GetModuleFileNameA(hInstance, pathBuf_0x4ED308, 64)
    find last '\\' in path -> truncate to directory
    call SetupPaths(hInstance, hPrevInstance)    // 0x401163

    // Setup game timer (10ms interval = ~100 ticks/sec)
    timerCallback = 0x401389
    SetTimer(hWnd, 1, 10, timerCallback)        // IAT 0x69370C
    if (timer == 0):
        MessageBoxA("Too many clocks or timers!")
        ExitProcess(1)

    // If already at 640x480, use MoveWindow to resize
    if (already640x480):
        MoveWindow(hWnd, 0, 0, 640, 480, TRUE)  // IAT 0x693710
    else:
        call ChangeDisplayMode()                 // 0x401771

    ShowWindow(hWnd, SW_SHOW)                   // IAT 0x693714
    UpdateWindow(hWnd)                          // IAT 0x693718

    // Load cursor resources
    for each cursor in cursor_table at 0x4F2B88:
        LoadCursorA(hInstance, cursorID) or LoadCursorA(NULL, IDC_ARROW/IDC_CROSS)
        store in cursor array at 0x4F67E0

    // Display resolution change if not 640x480
    call ChangeDisplaySettingsA(devmode, 0)     // for 640x480x8
```

**Key Global Variables - Initialization:**

| Address | Type | Purpose |
|---------|------|---------|
| 0x680F54 | HWND | Main application instance handle |
| 0x68FF94 | HWND | Main window handle |
| 0x68FF90 | HDC | Main window device context |
| 0x68FF0C | short | Screen bits per pixel |
| 0x68C2E0 | short | Screen color planes |
| 0x68B298 | short | Screen width (horizontal resolution) |
| 0x6815CA | short | Screen height (vertical resolution) |
| 0x69017E | short | Flag: already at 640x480 |
| 0x68C3E0 | DWORD | Startup timestamp (timeGetTime) |
| 0x4ED308 | char[64] | Application directory path |

### 1.3 Window Class & WndProc

**Window Class Name:** `"UUUUVKClass"` (string at runtime)

The window is created with `CreateWindowExA` at `0x4BD42A`:

```
function CreateGameWindow(hInstance):
    wndclass.lpfnWndProc = WndProc              // stored at 0x4F337C
    wndclass.lpszClassName = "UUUUVKClass"      // 0x4ED2F0
    RegisterClassA(&wndclass)                   // IAT 0x6936F8
    hWnd = CreateWindowExA(
        WS_EX_OVERLAPPEDWINDOW,                 // 0x82000000
        "UUUUVKClass",                          // 0x4ED2F0
        className,
        0, 0,                                   // x, y
        screenWidth, screenHeight,
        NULL, hInstance, NULL
    )
    store hWnd at 0x68FF94
    hDC = GetDC(hWnd)                           // IAT 0x693724
    store at 0x68FF90
```

### 1.4 WndProc (0x4BC661 area, ret $0x10 at 0x4BF041)

The WndProc handles Windows messages via a jump table. Key message handlers:

```
function WndProc(hWnd, msg, wParam, lParam):
    switch (msg):
        case WM_PAINT (0x0F):
            BeginPaint / EndPaint
            
        case WM_KEYDOWN (0x100):
            extract scancode from lParam high byte
            store keycode at 0x680F28
            
        case WM_KEYUP (0x101):
            extract scancode, set key-up flags
            
        case WM_MOUSEMOVE (0x200):
            mouseX = LOWORD(lParam) - windowOffsetX    // 0x680F4E
            mouseY = HIWORD(lParam) - windowOffsetY    // 0x680F50
            store at 0x68FA30 (mouseX), 0x68FA32 (mouseY)
            hWnd stored at 0x68FA34
            
        case WM_LBUTTONDOWN (0x201):
            store mouse pos, set click flags
            mouseClickFlags at 0x68FA38..0x68FA48
            
        case WM_RBUTTONDOWN (0x204):
            store mouse pos, set right-click flags
            
        case WM_LBUTTONUP (0x202):
            release capture, set button-up flags
            
        case WM_TIMER (0x113):
            if (timerID == 1): call GameTick()   // 0x401389 -> 0x4BC661
            
        case WM_DESTROY (0x02):
            set quit flag 0x68C324 = 0
            cleanup palette, WinG resources
            PostQuitMessage(0)
            
        default:
            DefWindowProcA(hWnd, msg, wParam, lParam)
```

**Key Global Variables - Input:**

| Address | Type | Purpose |
|---------|------|---------|
| 0x68FA30 | short | Current mouse X (client coords) |
| 0x68FA32 | short | Current mouse Y (client coords) |
| 0x68FA34 | HWND | Window receiving mouse input |
| 0x68FA38 | short | Left button state flag |
| 0x68FA3A | short | Left button state flag 2 |
| 0x68FA3E | short | Right button modifier |
| 0x68FA40 | short | Right button click flag |
| 0x68FA42 | short | Left button double-click flag |
| 0x68FA44 | short | Button release flag |
| 0x68FA46 | short | Button press flag |
| 0x68FA48 | short | Right button press flag |
| 0x680F28 | short | Last key scancode |
| 0x680F4E | short | Window client area X offset |
| 0x680F50 | short | Window client area Y offset |

### 1.5 Main Game Loop

Two message pump patterns were identified. The primary loop at `0x4BDFCF`:

```
function MainGameLoop():
    call PreLoopInit()              // 0x402644
    g_inputEnabled = 0              // 0x6813B8
    call AudioInit()                // 0x402761
    g_quitFlag = 0                  // 0x680F40

    while (true):
        if PeekMessageA(&msg, NULL, 0, 0, PM_REMOVE):  // IAT 0x6936A4
            if (!TranslateAccelerator(hWnd, hAccel, &msg)):  // IAT 0x6936E4
                if (msg.message == WM_QUIT):    // 0x12
                    call Shutdown()              // 0x4018C5
                TranslateMessage(&msg)           // IAT 0x6936E8
                DispatchMessage(&msg)            // IAT 0x6936EC

        if (g_quitFlag != 0): break

        // Per-frame game logic
        if (g_gameActive):
            call UpdateLogic()       // 0x4027C5
        call RenderFrame()           // 0x4020A9 -> 0x44123A
        call ProcessInput()          // 0x401672
        call FlipBuffers()           // 0x4020E5
        call HandleAudio()           // 0x401753
```

A secondary tight loop exists at `0x4BE09E` for modal/blocking game states:

```
function ModalLoop():
    while (g_gameActive != 0):       // 0x68C324
        PeekMessageA(...)
        if message available:
            TranslateAccelerator / TranslateMessage / DispatchMessage
        // no game logic updates - just processes messages
```

**Key Global Variables - Game Loop:**

| Address | Type | Purpose |
|---------|------|---------|
| 0x68C324 | short | Game active/running flag (0 = quit) |
| 0x68C328 | DWORD | Accelerator table handle |
| 0x680F40 | short | Quit requested flag |
| 0x6813B8 | byte | Input enabled flag |

---

## 2. Palette System

### 2.1 Palette Architecture

The game uses a **256-color 8-bit paletted display mode** via WinG. Palettes are loaded from `wow.pal` with a fallback to embedded PCX palette data.

### 2.2 Palette File Loading (0x410161)

The palette loading function iterates through drive letters to find the game data:

```
function LoadPalette():
    paletteFound = 0                            // -0xC(%ebp)
    currentDrive = 'C'                          // 0x43 = start from C:

    for drive = 'C' to '\\' (0x5C):            // iterate drive letters
        pathBuffer[0] = drive                   // store at 0x4ED398
        strcat(pathBuffer_0x681600, driveLetter)        // 0x4D4BA0
        strcat(pathBuffer_0x681600, ":\\path\\")        // 0x4ED348
        strcat(pathBuffer_0x681600, "wow.pal")          // 0x4F2B60

        fileHandle = _lopen(pathBuffer, OF_READ)        // IAT 0x693528
        if (fileHandle > 0):
            paletteFound = drive
            _lclose(fileHandle)                         // IAT 0x6935FC
            currentDrive = 0x63 ('c')                   // reset loop
        else:
            call FallbackPalette()                      // 0x4020D1 -> 0x4BE08E

    if (paletteFound == 0):
        clear drive byte at 0x4ED398
        return 0
    return 1
```

**Observed behavior:** The game searches drives C: through Z: (and beyond) looking for `wow.pal`. This is the CD-ROM detection mechanism - it tries each drive letter until it finds the game data.

### 2.3 Palette Data Format

The palette data is stored as **RGBQUAD arrays** (4 bytes per entry: B, G, R, reserved) in memory, matching the Windows BITMAPINFO format used by WinG:

```
function StorePalette(paletteSource):
    // Copy from source RGBQUAD format to internal palette storage
    for i = 0 to 255:                          // 0x100 entries
        palette_R[i] = source[i*4 + 0]         // 0x68D704 + i*4 = R
        palette_G[i] = source[i*4 + 1]         // 0x68D705 + i*4 = G
        palette_B[i] = source[i*4 + 2]         // 0x68D706 + i*4 = B

    // Animate system palette (reserve entries 1-254, keep 0 and 255)
    AnimatePalette(hSystemPalette, 1, 254, paletteEntries)  // IAT 0x6934C0
```

**Key observation:** The source data at `0x68D704` uses 4-byte RGBQUAD stride. The palette entries are stored R, G, B in separate byte offsets within each 4-byte slot. Index 0 and 255 are reserved (system colors).

### 2.4 WinGSetDIBColorTable Calls (0x40DAF0)

After storing the palette, the code applies it to **all WinG surfaces**:

```
function ApplyPaletteToSurfaces():
    // Remap palette from RGBQUAD internal storage to WinG RGBQUAD format
    for i = 0 to 255:
        wingPalette[i].rgbRed   = palette_R[i]      // 0x68FADA + i*4
        wingPalette[i].rgbGreen = palette_G[i]       // 0x68FAD9 + i*4
        wingPalette[i].rgbBlue  = palette_B[i]       // 0x68FAD8 + i*4

    // Apply to all 5 WinG bitmap surfaces:
    WinGSetDIBColorTable(wingDC_back,  0, 256, paletteRGBQUAD)  // 0x68F950
    WinGSetDIBColorTable(wingDC_front, 0, 256, paletteRGBQUAD)  // 0x68C2F0
    WinGSetDIBColorTable(wingDC_3,     0, 256, paletteRGBQUAD)  // 0x6815D0
    WinGSetDIBColorTable(wingDC_4,     0, 256, paletteRGBQUAD)  // 0x68FEE0
    WinGSetDIBColorTable(wingDC_5,     0, 256, paletteRGBQUAD)  // 0x68BE90
```

### 2.5 Palette Bit Depth

**The palette is 8-bit per channel** (standard Windows RGBQUAD, 0-255 range). This is NOT 6-bit VGA palette (0-63 range). The `AnimatePalette` call with `peFlags` and the direct RGBQUAD storage confirm full 8-bit values.

However, the `wow.pal` file itself may contain 6-bit VGA values (0-63) that get scaled to 8-bit. This would need runtime verification. The `COLOR_64` and `COLOR_256` strings found in the executable suggest the engine handles both VGA 6-bit and standard 8-bit palettes (matching FLC/FLI format conventions where COLOR_64 is the FLI 6-bit palette chunk and COLOR_256 is the FLC 8-bit palette chunk).

### 2.6 PCX Palette Fallback

When `wow.pal` is not found, the code jumps to `0x4BE08E` which is a minimal message loop handler. The actual PCX palette extraction would occur during PCX image loading, where the last 769 bytes of a PCX file (signature byte 0x0C followed by 256 RGB triplets) contain the palette.

**Key Global Variables - Palette:**

| Address | Type | Purpose |
|---------|------|---------|
| 0x68D700 | struct | Palette header/metadata |
| 0x68D704 | byte[256*4] | Internal palette storage (RGBQUAD) |
| 0x68FAB0 | struct | WinG BITMAPINFO structure (header + palette) |
| 0x68FAD8 | byte[256*4] | WinG-format palette (RGBQUAD for SetDIBColorTable) |
| 0x68FED4-D7 | byte[4] | Special palette entries (0xFF, 0xFF, 0xFF, 0x01) = white + flag |
| 0x68CFAC | HDC | GDI palette DC |
| 0x4ED398 | char | Current drive letter for data search |
| 0x4ED384-390 | byte[5] | Drive letters for different data paths (bitmask-indexed) |

---

## 3. WinG Rendering System

### 3.1 WinG Surface Creation

The game creates multiple WinG offscreen surfaces for double-buffering and compositing:

```
function CreateWinGSurfaces():
    // Surface 1 (back buffer)
    width1 = (short)0x4F291C                    // display width
    height1 = (short)0x4F291E
    bytesPerLine = (short)0x68FEDC              // bytes per scanline
    bufferSize = height1 * bytesPerLine         // stored 0x68FAB8

    wingDC_1 = WinGCreateDC()                   // 0x4D4B9A -> 0x68C2F0
    if (wingDC_1 == NULL): error and exit
    
    wingBitmap_1 = WinGCreateBitmap(wingDC_1, &bitmapInfo, &pBits)  // 0x4D4B94
    store DC at 0x68C2F0, bitmap at 0x68C2F8, bits at 0x68C2FC
    
    hOldBitmap = SelectObject(wingDC_1, wingBitmap_1)   // IAT 0x6934D8
    store old bitmap at 0x68C2FC
    
    // Copy BITMAPINFOHEADER fields
    biWidth at 0x68C308, biHeight at 0x68C30A
    biBitCount = -1 (0xFFFF) at 0x68C30C        // special flag
    
    // Surface 2 (work buffer) - same pattern with different dimensions
    width2 = (short)0x4F2924
    wingDC_2 at 0x68F950, bitmap at 0x68F958
    
    // Surface 3-5 created similarly
    // Surface addresses: 0x6815D0, 0x68FEE0, 0x68BE90
```

### 3.2 WinG Blitting

`WinGBitBlt` (thunk at `0x4D4B88`) is called extensively throughout rendering. Major call sites include:

- `0x40E414` - Primary screen blit (back buffer to screen)
- `0x40E448` - Secondary blit operation
- `0x40E694` - Sprite compositing blit
- `0x40E923` - UI overlay blit
- `0x40EBB0` - Map rendering blit
- `0x4134A2` - Sprite/object rendering
- `0x413FEC` - Additional compositing

The WinGBitBlt signature is: `WinGBitBlt(destDC, destX, destY, width, height, srcDC, srcX, srcY)`

### 3.3 WinGRecommendDIBFormat

Called at `0x4BD536` during initialization to query the optimal DIB format for the display driver. This determines the byte ordering and stride for the WinG bitmaps.

---

## 4. Tile/Map Rendering

### 4.1 Cell Data Architecture

The map system supports up to **10,080 cells** (0x2760). Each cell's data is distributed across **5 parallel arrays** of 4 bytes each, stored at fixed global addresses:

| Array Address | Size | Content |
|--------------|------|---------|
| 0x59D8C0 | 10080 * 4 = 40,320 | Cell Word 1: tile indices + flags (primary terrain) |
| 0x5A7640 | 10080 * 4 = 40,320 | Cell Word 2: secondary terrain/overlay indices + flags |
| 0x5D07C0 | 10080 * 4 = 40,320 | Cell Word 3: passability/terrain type bits |
| 0x5DA540 | 10080 * 4 = 40,320 | Cell Word 4: height map / elevation data |
| 0x5E42C0 | 10080 * 4 = 40,320 | Cell Word 5: object/entity references |

**Total map data in memory: ~200KB**

### 4.2 Tile Index Encoding (0x41AF7B)

The tile index formula is **more complex than previously documented**. The function at `0x41AF00` packs/unpacks cell data using a multi-field bitpacking scheme:

#### Cell Word 1 Packing (0x59D8C0 array):

```
function PackCellWord1(cellIndex):
    temp = 0
    
    // Three 9-bit tile index fields, packed with SHL 9
    temp = (field_0x5EE060 & 0x1FF)             // tile_layer_0 (9 bits)
    temp = (temp << 9) | (field_0x5EE062 & 0x1FF)   // tile_layer_1 (9 bits)
    temp = (temp << 9) | (field_0x5EE064 & 0x1FF)   // tile_layer_2 (9 bits)
    
    // Five 1-bit flags
    temp = (temp << 1) | (field_0x5EE071 & 0x1)     // flag_A
    temp = (temp << 1) | (field_0x5EE072 & 0x1)     // flag_B
    temp = (temp << 1) | (field_0x5EE073 & 0x1)     // flag_C
    temp = (temp << 1) | (field_0x5EE074 & 0x1)     // flag_D
    temp = (temp << 1)                               // padding bit
    
    cellArray1[cellIndex] = temp
```

**Bit layout of Cell Word 1 (32 bits):**
```
[31..23] tile_layer_0 (9 bits - primary tile index, 0-511)
[22..14] tile_layer_1 (9 bits - secondary tile index, 0-511)
[13..5]  tile_layer_2 (9 bits - tertiary tile index, 0-511)
[4]      flag_A (wall/obstacle?)
[3]      flag_B (explored?)
[2]      flag_C (roof?)
[1]      flag_D (walkable?)
[0]      padding
```

#### Cell Word 2 Packing (0x5A7640 array):

```
function PackCellWord2(cellIndex):
    temp = 0
    
    // Two 9-bit fields with 4 interleaved flag bits
    temp = (field_0x5EE066 & 0x1FF)             // overlay_tile_0 (9 bits)
    temp = (temp << 9) | (field_0x5EE068 & 0x1FF)   // overlay_tile_1 (9 bits)
    
    // Then 4 more flag bits + another 9-bit field
    temp = (temp << 1) | (field_0x5EE06A & 0x1)     // flag_E
    temp = (temp << 1) | (field_0x5EE06B & 0x1)     // flag_F
    temp = (temp << 1) | (field_0x5EE06C & 0x1)     // flag_G
    temp = (temp << 1) | (field_0x5EE06D & 0x1)     // flag_H
    temp = (temp << 9) | (field_0x5EE06E & 0x1FF)   // overlay_tile_2 (9 bits)
    temp = (temp << 1)                               // padding
    
    cellArray2[cellIndex] = temp
```

#### Cell Word 3 - Terrain Type (0x5D07C0 array):

The third word encodes **terrain properties** as byte + 2-bit fields:

```
function UnpackCellWord3(cellIndex):
    temp = cellArray3[cellIndex]
    
    field_0x5EE070 = temp & 0xFF                 // terrain_base_type (8 bits)
    temp >>= 8
    field_0x5EE080 = temp & 0x03                 // terrain_modifier_0 (2 bits)
    temp >>= 2
    field_0x5EE07F = temp & 0x03                 // terrain_modifier_1 (2 bits)
    temp >>= 2
    field_0x5EE07E = temp & 0x03                 // terrain_modifier_2 (2 bits)
    temp >>= 2
    field_0x5EE07D = temp & 0x03                 // terrain_modifier_3 (2 bits)
    temp >>= 2
    field_0x5EE07C = temp & 0x03                 // terrain_modifier_4 (2 bits)
    temp >>= 2
    field_0x5EE07B = temp & 0x03                 // terrain_modifier_5 (2 bits)
    temp >>= 2
    field_0x5EE07A = temp & 0x03                 // terrain_modifier_6 (2 bits)
    temp >>= 2
    field_0x5EE079 = temp & 0x03                 // terrain_modifier_7 (2 bits)
    temp >>= 2
    field_0x5EE078 = temp & 0x03                 // terrain_modifier_8 (2 bits)
    temp >>= 2
    field_0x5EE077 = temp & 0x03                 // terrain_modifier_9 (2 bits)
    temp >>= 2
    field_0x5EE076 = temp & 0x03                 // terrain_modifier_10 (2 bits)
    temp >>= 2
    field_0x5EE075 = temp                        // terrain_modifier_11 (remaining)
```

This encodes **12 two-bit terrain modifiers per cell** (likely per-edge or per-corner passability/cover values for the isometric diamond) plus a base terrain type byte.

#### Cell Word 4 - Elevation (0x5DA540 array):

Elevation data uses **6-bit fields**:

```
function UnpackCellWord4(cellIndex):
    temp = cellArray4[cellIndex]
    
    field_0x5EE08C = temp & 0x3F                 // elevation_corner_0 (6 bits, 0-63)
    temp >>= 6
    field_0x5EE08A = temp & 0x3F                 // elevation_corner_1 (6 bits)
    temp >>= 6
    field_0x5EE088 = temp & 0x3F                 // elevation_corner_2 (6 bits)
    temp >>= 6
    field_0x5EE086 = temp & 0x3F                 // elevation_corner_3 (6 bits)
    temp >>= 6
    field_0x5EE084 = temp & 0x03                 // elevation_flags_0 (2 bits)
    temp >>= 2
    field_0x5EE083 = temp & 0x03                 // elevation_flags_1 (2 bits)
    temp >>= 2
    field_0x5EE082 = temp & 0x03                 // elevation_flags_2 (2 bits)
    temp >>= 2
    field_0x5EE081 = temp                        // elevation_flags_3 (remaining)
```

**Elevation encoding:** 4 corner heights as 6-bit values (0-63) = per-vertex elevation for the isometric diamond tile, plus 4 two-bit flags (slope type, cliff edges, etc.).

#### Cell Word 5 - Objects/Entities (0x5E42C0 array):

```
function UnpackCellWord5(cellIndex):
    temp = cellArray5[cellIndex]
    
    field_0x5EE096 = temp & 0xFF                 // object_id (8 bits)
    temp >>= 8
    field_0x5EE094 = temp & 0x3F                 // object_param_0 (6 bits)
    temp >>= 6
    field_0x5EE092 = temp & 0x3F                 // object_param_1 (6 bits)
    temp >>= 6
    field_0x5EE090 = temp & 0x3F                 // object_param_2 (6 bits)
    ...remaining bits for additional params
```

### 4.3 Temporary Cell Structure at 0x5EE060

All cell unpacking writes to a shared temporary structure:

| Offset | Type | Purpose |
|--------|------|---------|
| +0x00 (5EE060) | short | tile_layer_0 |
| +0x02 (5EE062) | short | tile_layer_1 |
| +0x04 (5EE064) | short | tile_layer_2 |
| +0x06 (5EE066) | short | overlay_tile_0 |
| +0x08 (5EE068) | short | overlay_tile_1 |
| +0x0A (5EE06A) | byte | overlay flag E |
| +0x0B (5EE06B) | byte | overlay flag F |
| +0x0C (5EE06C) | byte | overlay flag G |
| +0x0D (5EE06D) | byte | overlay flag H |
| +0x0E (5EE06E) | short | overlay_tile_2 |
| +0x10 (5EE070) | byte | terrain_base_type |
| +0x11 (5EE071) | byte | cell flag A |
| +0x12 (5EE072) | byte | cell flag B |
| +0x13 (5EE073) | byte | cell flag C |
| +0x14 (5EE074) | byte | cell flag D |
| +0x15..+0x20 | bytes | terrain modifiers (12 x 2-bit) |
| +0x21..+0x24 | bytes | elevation flags (4 x 2-bit) |
| +0x26..+0x2C | shorts | elevation corners (4 x 6-bit) |
| +0x30..+0x36 | shorts | object parameters |

### 4.4 MAP File Loading

The `.map` extension and `"maps"` directory string (line 7883) indicate map data is loaded from files. The file dialog filter string at line 8687 reveals the editor supports:

```
"Tile Map (*.map)|*.map|Flic (*.flc)|*.flc|Tile Set (*.til)|*.til|
 Object Set (*.obj)|*.obj|Tile Data (*.dat)|*.dat|"
```

Map files use the FLC-derived container format (magic `0xF1FA` detected at `0x4132C3`). The loading code at `0x413167`:

```
function LoadSpriteFile(filename):
    bufferSize = 0x7D000 = 512,000 bytes        // max allocation
    fileReady = 1

    if (g_fileLoadLock == 1): return 0           // 0x680F4C

    call OpenFile(filename, &ofstruct, OF_READ)  // 0x40269E wrapper

    // Read file header (0x84 = 132 bytes)
    _hread(fileHandle, headerBuf_0x690690, 0x84)
    
    spriteCount = header.numFrames               // offset +0x06
    remainingWidth = 640 - header.width          // 0x280 - width at +0x08
    centerOffsetX = remainingWidth / 2           // stored 0x690A8E
    remainingHeight = 480 - header.height        // 0x1E0 - height
    centerOffsetY = remainingHeight / 2          // stored 0x690A9A

    // Seek past header to frame data
    _llseek(fileHandle, header.dataOffset, SEEK_SET)

    // Read frame count
    spriteCount--
    while (spriteCount > 0):
        // Read 6-byte frame header
        _hread(fileHandle, frameBuf, 6)
        frameType = frameBuf[+0x04]              // 2 bytes at offset 4
        
        if (frameType == 0x10):                  // type 0x10 = has subheader
            frameBuf += 2                        // skip 2 extra bytes
        
        frameDataSize = frameBuf[0] - 6          // subtract header
        _hread(fileHandle, spriteDataPtr, frameDataSize)

        // Lookup decode function from table at 0x4ECEF0
        if (decodeFuncTable[frameType] == NULL): return 0
        
        // Call appropriate decoder
        call decodeFuncTable[frameType](spriteData)
        
        spriteCount--
```

**Identified frame types and their decoder strings:**
- `COLOR_256` - 8-bit palette chunk (FLC format)
- `COLOR_64` - 6-bit VGA palette chunk (FLI format)
- `DELTA_FLC` - Delta compression (FLC)
- `DELTA_FLI` - Delta compression (FLI)
- `BLACK` - Clear frame to black
- `BYTE_RUN` - ByteRun1/PackBits RLE compression

### 4.5 Isometric Projection Math

Evidence of isometric projection was found at `0x45FE72`:

```
function DetermineQuadrant(tileX, tileY):
    tileX = tileX & 0x7F                       // clamp to 0-127
    tileY = tileY & 0x3F                       // clamp to 0-63
    
    if (!(dirFlag1 & 1) && !(dirFlag2 & 1)):   // normal case
        threshold1 = (int)(tileX * CONST_A + CONST_B)  // fmull 0x4EB0A0 + faddl 0x4EB0A8
        threshold2 = (int)(tileX * CONST_C)            // fmull 0x4EB0B0
        
        if (tileY <= threshold1):
            subtract 0x47 from cell offset
            if (tileY <= threshold2):
                quadrant = 3                    // NW quadrant
            else:
                quadrant = 4                    // NE quadrant
        else:
            if (tileY <= threshold2):
                quadrant = 1                    // SW quadrant
            else:
                quadrant = 2                    // SE quadrant
```

The floating-point constants at `0x4EB0A0`, `0x4EB0A8`, `0x4EB0B0`, `0x4EB0B8` define the isometric diamond slopes. The values `0x47` and `0x45` subtracted from cell offsets are the tile width (71) and height (69) in the isometric diamond, confirming a tile geometry close to but not exactly 2:1 ratio.

The `SHL $0x6` (multiply by 64) operations found throughout tile rendering code (at `0x417F24`, `0x41815E`, etc.) suggest **64-pixel wide tile sprites** in the sprite sheets.

**Quadrant variable:** `0x4EEA2C` stores the mouse-over quadrant (1-4) for the current isometric tile.

### 4.6 Tile Sprite Sheet Indexing

The `shl $0x6` (x64) pattern is used to calculate sprite sheet byte offsets:

```
function GetTileSpriteOffset(tileIndex):
    return tileIndex * 64                       // each tile is 64 bytes wide
```

The tile sprite data is loaded from `Tiles.spr` and `obj1.spr` (strings found at lines 8563-8564).

---

## 5. Sprite/RLE System

### 5.1 FLC/FLI Container Format

Sprite files (`.obj`, `.spr`) use the **Autodesk FLC/FLI animation format** as their container. This was confirmed by:

1. The magic number check `0xF1FA` at `0x4132C3`
2. The presence of standard FLC frame type handlers: `COLOR_256`, `COLOR_64`, `DELTA_FLC`, `DELTA_FLI`, `BLACK`, `BYTE_RUN`
3. A 132-byte file header (`0x84` bytes read at `0x4131E5`)
4. Frame-by-frame structure with 6-byte frame headers

### 5.2 ByteRun1 (PackBits) RLE Compression

The `BYTE_RUN` string (line 7701) identifies the compression algorithm as **ByteRun1**, which is standard in FLC/FLI files:

```
// ByteRun1 algorithm (behavioral description):
function DecodeByteRun(input, output, width, height):
    for each scanline:
        bytesRemaining = width
        while bytesRemaining > 0:
            controlByte = readByte(input)
            if controlByte > 128:
                // Run of identical bytes
                runLength = 256 - controlByte + 1
                value = readByte(input)
                for i = 0 to runLength-1:
                    output[pos++] = value
                bytesRemaining -= runLength
            else if controlByte < 128:
                // Literal copy
                copyLength = controlByte + 1
                for i = 0 to copyLength-1:
                    output[pos++] = readByte(input)
                bytesRemaining -= copyLength
            // controlByte == 128: NOP
```

### 5.3 Transparency Handling

Palette index 0 is used for transparency. The first palette entry is preserved as system black and skipped during blitting. The code at `0x68FED4-D7` sets special palette entries: `(0xFF, 0xFF, 0xFF, 0x01)` - likely a white transparency marker.

### 5.4 Sprite Data Flow

```
function LoadObject(filename):
    // Build full path
    strcat(appPath, filename)
    
    // Open as FLC file
    fileHandle = OpenFile(path, &ofstruct, OF_READ)
    if (fileHandle <= 0):
        OutputDebugString("Object not loaded Function: LoadObject")
        return 0
    
    // Read FLC header
    _hread(fileHandle, headerBuf, 0x84)         // 132-byte FLC header
    
    // Read sprite data pointer
    spriteDataPtr = globalSpriteBuffer           // 0x690758
    
    // Read all frames
    _hread(fileHandle, spriteDataPtr, frameDataSize)
    
    // Store data pointer for frame decoding
    currentSpritePtr = spriteDataPtr             // 0x69058C
    
    // Each sprite is accessed by reading 2 bytes (width, height) then pixel data
    spriteWidth = readByte() | (readByte() << 8)  // LE 16-bit
```

The global sprite data pointer at `0x69058C` is used as a streaming cursor - incremented byte-by-byte during sprite data parsing.

### 5.5 Sprite Mirroring

No explicit mirror/flip instructions were identified in the sprite loading code. Mirroring is likely handled during rendering by iterating pixels in reverse X order (right-to-left for horizontal flip). The direction-facing animations may be stored as separate frames rather than computed via mirroring.

### 5.6 Function Names from Debug Strings

| Function | Error String |
|----------|-------------|
| LoadObject | "Object not loaded Function: LoadObject" |
| LoadObject | "Index problem Function: LoadObject" |
| LoadObject | "Can't load sprites Function: LoadObject" |
| LoadObject | "cant close file Function: LoadObject" |
| LoadSprites | "Sprites not loaded Function: LoadSprites" |
| LoadSprite | "Index problem Function: LoadSprite" |
| LoadSprite | "Can't load sprites Function: LoadSprite" |
| LoadSprite | "cant close file Function: LoadSprite" |
| LoadDataToSlot | "Function: LoadDataToSlot - File not open" |
| LoadDataToSlot | "Function: LoadDataToSlot - Could not load sprites into data slot." |
| SetupSprite | "!File not found: Function SetupSprite" |
| LoadAnimationData | "cound not load file Function: LoadAnimationData" |
| OpenAnimationDataFile | "Function:OpenAnimationDataFile - Could not Open File" |
| CloseAnimationDataFile | "Function: CloseAnimationDataFile - File not open" |

---

## 6. File I/O

### 6.1 Text DAT File Parsing

All `.dat` text files are parsed using C standard library `sscanf()` with format strings. The format strings reveal the exact field layout of each data file:

#### weapons.dat Format

```
Format: "%s %hd %hd %hd %hd %hd-%hd %hd %hd %hd %hd %hd %hd %hd %s %hd"

Fields (16 total):
  1. weaponname (string)
  2-5. four short integers (likely: type, caliber, weight, cost)
  6-7. damage range (min-max, with literal '-' separator)
  8-14. seven short integers (range, accuracy, rate_of_fire, clip_size, burst_count, AP_cost, etc.)
  15. ammoname (string)
  16. one short integer (ammo_type_id?)

Validation: "Weapon[i].weaponname too long"
            "Weapon[i].ammo.ammoname too long"
```

#### mercs.dat Format

```
Parsed with multiple sequential sscanf calls:
  1. "%hd"     - merc count or ID
  2. "%hd %hd" - two shorts (nationality? gender?)
  3-10. "%hd"  - individual stat fields (8 stats)
  11. "%ld%ld%ld" - three longs (salary? contract terms?)
  12. "%hd"    - additional field

Validation: "MercRolodex[i].nationality too long"
            "MercRolodex[i].name too long"
            "MercRolodex[i].nickname too long"
            "Too many mercs for rolodex"
```

#### equip.dat Format

```
  "%hd" and "%ld" alternating fields
  Error: "Could not open file"
```

#### target.dat Format

```
Format: "%hd %hd %hd %hd %hd %hd %hd %hd %hd %hd %hd %hd %hd %hd %hd %hd %hd %hd %hd %hd"

20 short integers per record - likely hit location table:
  [body_part_0_chance ... body_part_19_chance]
  
Or target profile: range brackets, stance modifiers, terrain modifiers, etc.
```

#### Mission Data (mssn{NN}.dat) Format

Complex multi-line format with mixed types:

```
Line 1:  "%hd %hd"                                          - mission ID, type
Line 2:  "%ld %ld %hd %hd"                                  - budget, reward, min_mercs, max_mercs
Lines 3-6: "%ld %ld %ld %ld"                                - four sets of 4 longs each
Line 7:  "%hd %hd %hd %hd"                                  - four shorts
Line 8:  "%ld %ld %ld %ld %hd %hd %hd %hd"                  - 4 longs + 4 shorts
Lines 9-12: "%hd %hd %hd %hd %hd %hd %hd %hd"               - four sets of 8 shorts each
Lines 13-15: "%ld %ld" (x3)                                  - pairs of longs
Line 16: "%hd %hd %hd %hd %hd %hd"                          - 6 shorts
Line 17-19: "%hd" (x3)                                      - individual shorts
Line 20: "%hd %hd %hd %hd %hd %hd %hd %hd %hd %hd %hd %hd %hd"  - 13 shorts (stats?)
Line 21: "%hd %hd %hd %hd %hd %hd"                          - 6 shorts
Lines 22-24: Various "%hd" patterns
Lines 25+: "%hd %hd %hd %hd %hd %hd"                        - coordinate/placement data
Line N: "%ld %ld %ld %hd %hd %hd"                           - economy values
Lines N+1-4: "%ld" (x4)                                     - additional long values
```

#### moves.dat (AI Movement Grid)

```
Header:
  "%hd"          - grid dimension or count
  "%hd"          - secondary count
  "%hd"          - alert level count
  "%hd"          - ???
  "%hd"          - ???
  "%hd %hd"      - grid dimensions

Grid data (per alert level, repeated up to 6 times):
  "%hd %c %hd %hd %c %hd %hd %c %hd %hd %c %hd %hd %c %hd %hd %c %hd %hd %c %hd %hd %c %hd %hd %c %hd %hd %c %hd %hd"
  
  = 11 repetitions of: short char short
  Interpretation: 11 waypoints per line, each with:
    - node_id (short)
    - direction_char (character: N/S/E/W/etc.)
    - next_node (short)

Post-grid:
  "%ld %hd"      - timing/weight values
```

#### ainode.dat

Referenced at lines 8469, 8725. Loaded with `fopen()` in `"r+b"` mode (read+binary), suggesting a mixed text/binary format. The buffer size limit is 2999 bytes ("File Larger than buffer size.. > 2999").

### 6.2 Binary File Loading

Binary files use the Win16-era file API:

```
// File I/O Pattern:
fileHandle = _lopen(path, mode)          // or OpenFile()
_hread(fileHandle, buffer, byteCount)    // read data
_llseek(fileHandle, offset, origin)      // seek
_lclose(fileHandle)                      // close
```

**Note:** The game uses `_lopen`/`_hread`/`_lclose` (16-bit API wrappers) rather than `CreateFileA`/`ReadFile`. This confirms the codebase was ported from 16-bit Windows.

### 6.3 Path Resolution

```
function BuildFilePath(filename):
    // Base path stored at 0x4ED308 (extracted from GetModuleFileNameA at startup)
    // Secondary path buffer at 0x681600

    fullPath = appDirectory + "\\" + subdirectory + "\\" + filename
    
    // Drive letter at 0x4ED398 determines data source
    // Maps directory: "maps" subdirectory
    // Sprite data: loaded relative to app path
```

The registry key `"SOFTWARE\\Random Games\\Wages\\1.0"` is queried for the install path.

### 6.4 Settings Persistence

Settings are saved via Windows INI file API:
- `GetPrivateProfileStringA` (IAT 0x693520) - read settings
- `WritePrivateProfileStringA` (IAT 0x693524) - write settings

Calls observed at `0x4B9D66..0x4B9DD8`, reading/writing to `settings.dat` (used as an INI file despite the `.dat` extension).

### 6.5 Save/Load System

```
Save files: "save" + number + ".dat" (e.g., save1.dat)
Save data:  "savedata.dat" - serialized game state
Settings:   "settings.dat" - INI-format configuration
Turn save:  "TurnSave.dat" - mid-combat save state
```

---

## 7. Combat System

### 7.1 Turn Structure

The `TurnSave.dat` file reference confirms turn-based save/restore. The debug cheat codes found in strings reveal the combat state machine:

| Cheat Code | Effect | Implies |
|------------|--------|---------|
| "DEADMAN" | "All enemies are dead" | Kill all hostiles |
| "KILLDOG" | "Dead Dogs" | Kill animal units |
| "OH DARN" | "Incoming....." | Trigger incoming fire event |
| "ELBOW ROOM" | "Ap points increased" | Action Points are a resource |
| "MORTAL" | "Merc returned to a mortal" | God mode toggle |
| "STATS" | "Stats increased by 10 pts" | Stat values are integers |
| "SMOKE" | "25 Smoke grenades given" | Smoke grenades exist |
| "FILL MAGAZINE" | "Magazine items in abdules purchased" | Ammo/magazine system |
| "HOUR" | "Hour added to time" | Time tracking in hours |
| "MMIN" | "5 min added to mission time" | 5-minute time increments |
| "SET MINE" | "Mine has been set 8)" | Mine placement mechanic |
| "911" | "10 firstaid kits given" | Medical items |

### 7.2 Target Resolution (target.dat)

The `target.dat` file contains **20 short integers per record**. This is likely a **hit location probability table** or **accuracy modifier table**:

```
Possible interpretation:
  target.dat record = 20 shorts:
  [range_0_mod, range_1_mod, ..., range_9_mod,    // 10 range brackets
   stance_standing, stance_kneeling, stance_prone,  // 3 stance mods
   cover_none, cover_partial, cover_full,           // 3 cover mods
   terrain_open, terrain_forest, terrain_urban, terrain_water]  // 4 terrain mods
```

### 7.3 Weapon System

Weapons have primary and secondary slots (WEAPON and WEAPON2 strings appear frequently paired). The weapon data structure includes:

- Weapon name (string)
- Numeric stats: type, caliber, weight, cost, damage_min, damage_max, range, accuracy, rate_of_fire, clip_size, burst_count, AP_cost, additional_stat
- Ammo name (string)
- Ammo type reference

Equipment categories from the order system:
- WEAPON (primary)
- WEAPON2 (secondary/sidearm)
- AMMO
- EQUIPMENT

### 7.4 Action Point System

The "Ap points increased" cheat confirms AP as the core action economy. The `%hd` (short integer) format for most combat values means APs are 16-bit signed integers.

### 7.5 Medical System

The "911" cheat gives "10 firstaid kits" - medical supplies are inventory items with quantity tracking. "Health increased for current merc" from the "NOUN" cheat confirms per-merc health tracking.

### 7.6 Mission Summary / Economy

The mission summary screen displays:

```
MISSION SUMMARY
  INCOME:
    Contract:
    Bonus:
    TOTAL:
  EXPENSES:
    Personnel:
    Hiring:
    Bonuses:
    Deaths:
    Medical:
    Equipment:
    Weapons Lease:
    Ammunition:
    Equipment:
    Lost Weapons:
    Returned Ammo:
    Returned Equipment:
    Found Weapons:
    Intelligence:
    Travel:
    Training:
    Overhead:
    Loan & Grift:
    Miscellaneous:
    TOTAL:
  PROFIT(LOSS):

MERCENARY SUMMARY
  NAME | RATING | STATUS | KIA | WIA | MIA

COMPETITOR SUMMARY
COMPANY ASSETS
POPULARITY POLL
```

This reveals:
- Weapons are leased, not purchased outright
- Returned ammo/equipment is tracked (partial refunds)
- Lost weapons incur costs
- Intelligence and travel are expense categories
- Mercs can be KIA (killed), WIA (wounded), or MIA (missing)
- Competitor companies exist with their own ratings
- A popularity/reputation system tracks company standing

---

## 8. AI System

### 8.1 AI Node Graph (ainode.dat)

AI pathfinding uses a **node-based graph** system:

```
// Node path data format
"%d %d %d %d %d %d %d"    // 7 integers per node

Error: "NODEPATH Error loading file.."
Debug: "Node paths" (activated by "NODES" cheat)
```

The `ainode.dat` file is loaded in binary mode (`"r+b"`) with a maximum buffer size of 2,999 bytes, suggesting a compact format.

### 8.2 AI Movement Grid (moves.dat)

The `moves.dat` format encodes **movement orders per alert level**:

```
Structure:
  Header:
    grid_dimension_1 (short)
    grid_dimension_2 (short)
    num_alert_levels (short)
    param_1 (short)
    param_2 (short)
    grid_width, grid_height (short, short)

  Per alert level:
    Grid of waypoint chains:
      11 entries per line, each: node_id direction_char next_node
      
      Direction characters likely: N, S, E, W, or hex direction codes
      
    Followed by: timing_value (long), weight (short)

  Repeated for each alert level (up to 6 levels observed from format string repetitions)
```

The 6 repetitions of the grid format string (with `%hd %c %hd` x 11 pattern) suggest **6 alert levels**, each with its own patrol/movement grid.

### 8.3 AI Decision Flow

Based on the data structures:

```
function AITick(unit):
    alertLevel = getCurrentAlertLevel(unit)     // 0-5
    moveGrid = loadMoveGrid(alertLevel)
    
    currentNode = unit.currentNodeId
    nextNode = moveGrid[currentNode].nextNodeId
    direction = moveGrid[currentNode].direction
    
    // Follow node chain based on alert level
    moveToward(nextNode, direction)
```

The debug string `"ALERT THE PROGRAMMERS! - CHECKPOINT N"` (N = 1, 2, 4, 5, 8) suggests internal state machine checkpoints, possibly for debugging AI state transitions.

---

## 9. Audio System

### 9.1 MIDI Playback

MIDI playback uses the **MCI (Media Control Interface)** via `mciSendCommandA`:

```
function PlayMidiSound(midiDeviceId):
    // Open MIDI device
    mciOpenParms.device = midiDeviceId & 0xFFFF
    result = mciSendCommandA(deviceId, MCI_OPEN, 0x808, &mciOpenParms)
    
    if (result != 0):
        call ReportMCIError(result)              // 0x402478
        return 0
    return 1

Errors:
    "MIDI MAPPER Port INVALID."
    "MIDI MAPPER BAD PLAY .. CALL FITZ."         // developer name: Fitz
```

MIDI files use `.mid` extension. The system checks `system.ini` for the `mciseq.drv` MIDI sequencer driver and a `disablewarning` flag.

The `mciGetDeviceIDA` function is used to look up the "sequencer" device by name.

### 9.2 CD Audio

CD audio playback uses MCI with the `"cdaudio"` device string. The `cdcfg.dat` file stores CD-ROM configuration including CD speed testing ("Testing CD-ROM Speed").

### 9.3 WAV Sound Effects

WAV playback uses two methods:

1. **sndPlaySoundA** (IAT 0x69379C) - Simple sound playback, called at `0x427A64` and nearby. The `wages.wav` file is referenced for startup sound.

2. **waveOutOpen** (IAT 0x6937A4) - Direct waveform output for streaming audio.

Volume control:
- `waveOutSetVolume` / `waveOutGetVolume` for WAV volume
- `midiOutSetVolume` / `midiOutGetVolume` for MIDI volume

### 9.4 VLA/VLS Audio Files

VLA (Voice/Language Audio?) files are referenced in cutscene/dialogue data:

```
" vla = %s"    // printed during scene loading (lines 7653, 7678, 7687)
" wav = %s"    // paired with VLA references
" obj = %s"    // object file for visual

".vls" extension found at line 8718
"Could Not Load File" / "Could not Read File" errors near VLS loading
```

VLA files appear to be audio data paired with sprite objects for animated cutscenes (character phone calls, briefings). VLS may be a VLA script/playlist format.

### 9.5 AVI Video

The game supports AVI video playback (via `avivideo` MCI device):

```
Videos referenced:
  nwlogo.avi      - New World Computing logo
  random.avi      - Random Games logo  
  wowlogo.avi     - Wages of War title
  opening.avi     - Opening cinematic
  ending.avi      - Ending cinematic
  credits.avi     - Credits roll
  
320x resolution variants:
  320nwrld.avi, 320rand.avi, 320logo.avi, 320open.avi
```

---

## 10. Game Structure

### 10.1 Mission List

17 missions identified (missn01 through missn17), with multiple briefing variants per mission:
- `mishn{N}a` - Briefing variant A (pre-mission?)
- `mishn{N}b` - Briefing variant B (success?)
- `mishn{N}c` - Briefing variant C (failure?)

Special missions: `mishn15i` (special ending?), `14win`/`14lose` (mission 14 outcomes)

### 10.2 Character Scenes

Named characters with sprite objects and audio:
- **Artie** - 12 scene variants (artie01-artie12) - likely the handler/broker
- **Vinnie** - 3 variants (vinnie01-vinnie03) - contact/informant
- **Mom** - 2 variants (mom01-mom02) - character
- **Pizza** - delivery scene
- Various `"lose"` and `"suc"` (success) outcome scenes

### 10.3 UI System

Button files (`.btn`):
- `main.btn`, `main2.btn`, `main3.btn` - Main menu button layouts
- `mantoman.btn` - Combat interface buttons
- `armexc.btn` - Arms exchange/equipment buttons

Cursor sprites: `cursors.spr`
Font files: `icfont10.chr`, `icfont12.chr`, `icfont18.chr`, `icfont24.chr`, `icfont30.chr` - 5 font sizes

### 10.4 Inventory/Store System

The order/purchase system uses a catalog UI:
- `catalog.obj` - Catalog sprite data
- `catmas01..03` - Catalog masks
- `opncat01..03` - Open catalog pages
- Categories: WEAPON, WEAPON2, AMMO, EQUIPMENT
- Display format: `" Description                Quantity   Total"`
- Total line: `"$ %ld"` format

The store is run by characters named `"abduls"`, `"lock"`, and `"serg"` (Abdul's, Lock's, and Serg's shops?).

### 10.5 Registry Key

```
HKEY_LOCAL_MACHINE\SOFTWARE\Random Games\Wages\1.0
```

Used for install path detection and settings storage on Windows NT.

---

## 11. Key Memory Map Summary

### Code Segments

| Range | Purpose |
|-------|---------|
| 0x401000-0x402700 | Jump table / dispatch thunks |
| 0x402700-0x40C000 | Core game logic, rendering helpers |
| 0x40C000-0x410000 | Palette, file I/O, sprite loading |
| 0x410000-0x420000 | Map/tile system |
| 0x420000-0x440000 | Game mechanics, combat |
| 0x440000-0x460000 | UI screens, menus, equipment |
| 0x460000-0x480000 | More UI, animation |
| 0x480000-0x4A0000 | Mission logic, AI |
| 0x4A0000-0x4B0000 | Economy, save/load |
| 0x4B0000-0x4C0000 | Audio, MIDI, video |
| 0x4BC000-0x4C0000 | Window management, WinG, initialization |
| 0x4D4000-0x4D6000 | IAT thunk functions |
| 0x4D7000-0x4E0000 | CRT startup, runtime |

### Data Segments

| Range | Purpose |
|-------|---------|
| 0x4EB000-0x4F4000 | String constants, FP constants, static data |
| 0x4F2B60 | "wow.pal" string |
| 0x4F291C-20 | Display dimension parameters |
| 0x4ED2F0 | Window class name "UUUUVKClass" |
| 0x4ED308 | Application directory path buffer |
| 0x4ECEF0 | FLC frame decoder function table |

### BSS / Global Variables

| Range | Purpose |
|-------|---------|
| 0x680000-0x690000 | Runtime state variables, input, flags |
| 0x690000-0x694000 | IAT, file buffers, sprite headers |
| 0x59D8C0-0x5EE100 | Map cell data arrays (5 x 10080 x 4 bytes) |

---

## 12. Implications for Engine Reimplementation

### Critical Findings

1. **Sprite format is FLC/FLI** - Not a custom format. Use the well-documented Autodesk FLIC specification for sprite loading. Frame types: BYTE_RUN (RLE), DELTA_FLC, DELTA_FLI, COLOR_256, COLOR_64, BLACK.

2. **Tile indexing is 3-layer** - Each cell has THREE 9-bit tile indices (primary, secondary, tertiary) plus overlay tiles. This is more complex than the initially assumed 2-layer system.

3. **Elevation is per-vertex** - Four 6-bit elevation values per cell (one per diamond corner) enable smooth terrain slopes, not just flat-topped tiles.

4. **Terrain has 12 modifier slots** - Twelve 2-bit fields per cell suggest per-edge passability and cover values around the isometric diamond.

5. **Palette is 8-bit channels** - Use standard 8-bit RGBQUAD, but support 6-bit VGA scaling for COLOR_64 chunks.

6. **Settings use INI format** - `GetPrivateProfileString` means settings.dat is Windows INI format despite the `.dat` extension.

7. **Weapons are leased** - The economy model includes weapon leasing, not outright purchase. Equipment costs are operating expenses.

8. **AI uses node-graph pathfinding with 6 alert levels** - Not free-form A* over the grid. Pre-computed patrol routes stored in `moves.dat`.

9. **Timer tick is 10ms** - The game runs at approximately 100 ticks/second via `SetTimer`.

10. **640x480 8-bit is mandatory** - The game requires exactly 640x480 resolution at 256 colors. No other modes are supported.

11. **Drive letter scanning for CD** - The game iterates C: through Z: looking for game data, which is the CD-ROM detection mechanism.

12. **WinG, not DirectDraw** - Rendering is entirely through WinG (predecessor to DirectDraw). Our SDL2 approach is a clean replacement.
