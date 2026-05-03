# FindWagesArtifacts.py — Ghidra headless script
#
# Purpose:
#   First-pass investigation of Wow.exe (Wages of War, 1996, MSVC 4.x x86).
#   Exports observational data we use to ground-truth the clean-room engine:
#     - all defined strings with addresses
#     - all functions with entry points + signatures
#     - all DLL imports
#     - strings filtered to ones that look like data-file path patterns
#       (TIL, OBJ, DAT, MAP, PCX, WAV, MID, COR, VLS, VLA, WRI, .EXE, MAPS\, scen, mish, mssn)
#     - for each filtered string, the list of functions that reference it (xrefs)
#
#   This is OBSERVATIONAL — facts about offsets and references in the binary.
#   It is NOT vendor source. Output goes to whiteroom/analysis/ and never
#   into crates/.
#
# Output:
#   <project_dir>/../analysis/Wow.exe-strings.json
#   <project_dir>/../analysis/Wow.exe-functions.json
#   <project_dir>/../analysis/Wow.exe-imports.json
#   <project_dir>/../analysis/Wow.exe-data-file-xrefs.json
#   <project_dir>/../analysis/Wow.exe-summary.md
#
# Usage (headless):
#   analyzeHeadless.bat <project_dir> <project_name> -process Wow.exe -postScript FindWagesArtifacts.py
#
#@category wages-of-war

import json
import os
import re

program = getCurrentProgram()
if program is None:
    print("FindWagesArtifacts: no program loaded")
    exit(1)

prog_name = program.getName()
listing = program.getListing()
fm = program.getFunctionManager()
st = program.getSymbolTable()
mem = program.getMemory()
ref_mgr = program.getReferenceManager()

# ---------- Output paths ----------
project_location = program.getDomainFile().getParent().getProjectLocator().getProjectDir()
analysis_dir = os.path.abspath(os.path.join(str(project_location), "..", "analysis"))
if not os.path.isdir(analysis_dir):
    os.makedirs(analysis_dir)

def out(name):
    return os.path.join(analysis_dir, prog_name + "-" + name)

# ---------- Helper: address -> containing-function name ----------
def func_at(addr):
    f = fm.getFunctionContaining(addr)
    return f.getName() if f else "<no_func>"

# ---------- Strings ----------
print("Collecting strings...")
strings_all = []
data_iter = listing.getDefinedData(True)
for d in data_iter:
    dt = d.getDataType()
    if dt is None:
        continue
    nm = dt.getName().lower()
    # Catch C strings, Unicode strings, string-array data types
    if "string" in nm or "char[" in nm or nm == "char":
        try:
            v = d.getValue()
            if v is None:
                continue
            s = str(v)
            if len(s) < 3:
                continue
            strings_all.append({
                "addr": "0x%s" % d.getAddress().toString(),
                "addr_offset": d.getAddress().getOffset(),
                "type": dt.getName(),
                "len": d.getLength(),
                "value": s,
            })
        except Exception:
            continue

print("  %d defined strings" % len(strings_all))

# ---------- Functions ----------
print("Collecting functions...")
funcs_all = []
for func in fm.getFunctions(True):
    entry = func.getEntryPoint()
    body_size = 0
    try:
        body_size = func.getBody().getNumAddresses()
    except Exception:
        pass
    funcs_all.append({
        "name": func.getName(),
        "entry": "0x%s" % entry.toString(),
        "entry_offset": entry.getOffset(),
        "size_addrs": int(body_size),
        "is_thunk": bool(func.isThunk()),
        "is_external": bool(func.isExternal()),
        "calling_convention": (
            func.getCallingConventionName() if func.getCallingConventionName() else ""
        ),
        "signature": func.getSignature().toString(),
    })
print("  %d functions" % len(funcs_all))

# ---------- Imports (DLL functions) ----------
print("Collecting imports...")
imports_by_dll = {}
for ext in st.getExternalSymbols():
    src = ext.getParentNamespace()
    dll = src.getName() if src else "<unknown>"
    imports_by_dll.setdefault(dll, []).append({
        "name": ext.getName(),
        "addr": "0x%s" % ext.getAddress().toString() if ext.getAddress() else None,
    })
print("  %d DLLs, %d total imports" % (
    len(imports_by_dll),
    sum(len(v) for v in imports_by_dll.values()),
))

# ---------- Data-file xref strings ----------
# These patterns target known WoW data-file references the engine expects.
# Wow.exe should contain printf/fopen format strings or filenames that the
# loader uses to find each scenario's TIL/OBJ/DAT/MAP/etc.
patterns = [
    ("TIL", re.compile(r"\.TIL\b|TIL\d|TILSCN", re.I)),
    ("OBJ", re.compile(r"\.OBJ\b|OBJ\d", re.I)),
    ("MAP", re.compile(r"\.MAP\b|SCEN.*MAP|MAPS[\\/]", re.I)),
    ("DAT", re.compile(r"\.DAT\b|TILES\d|JUNGSLD|RIFLWALK", re.I)),
    ("PCX", re.compile(r"\.PCX\b|OFFPIC|SCENPIC", re.I)),
    ("PAL", re.compile(r"\.PAL\b|PALETTE", re.I)),
    ("WAV", re.compile(r"\.WAV\b", re.I)),
    ("MID", re.compile(r"\.MID\b", re.I)),
    ("COR", re.compile(r"\.COR\b", re.I)),
    ("VLS", re.compile(r"\.VLS\b|\.VLA\b", re.I)),
    ("WRI", re.compile(r"\.WRI\b|MISHN|MSSN", re.I)),
    ("BTN", re.compile(r"\.BTN\b|BUTTONS", re.I)),
    ("CONTRACT", re.compile(r"CONTRACT|HIRE|MERC", re.I)),
    ("FORMAT_PRINTF", re.compile(r"%[-+ 0#]?[0-9]*[hl]?[a-zA-Z]")),
]

xref_strings = []
for s in strings_all:
    val = s["value"]
    matched_cats = []
    for (cat, rx) in patterns:
        if rx.search(val):
            matched_cats.append(cat)
    if not matched_cats:
        continue
    addr = toAddr(s["addr_offset"])
    refs = []
    for r in ref_mgr.getReferencesTo(addr):
        from_addr = r.getFromAddress()
        refs.append({
            "from": "0x%s" % from_addr.toString(),
            "from_offset": from_addr.getOffset(),
            "from_func": func_at(from_addr),
            "ref_type": r.getReferenceType().getName(),
        })
    xref_strings.append({
        "addr": s["addr"],
        "addr_offset": s["addr_offset"],
        "value": val,
        "categories": matched_cats,
        "xref_count": len(refs),
        "xrefs": refs,
    })

# Sort by xref_count descending — the most-referenced data-file strings
# are the high-value investigation targets.
xref_strings.sort(key=lambda x: -x["xref_count"])
print("  %d data-file-related strings with xrefs" % len(xref_strings))

# ---------- Write JSON outputs ----------
def write_json(name, data):
    p = out(name + ".json")
    with open(p, "w") as f:
        json.dump(data, f, indent=2)
    print("  wrote %s" % p)

write_json("strings", strings_all)
write_json("functions", funcs_all)
write_json("imports", imports_by_dll)
write_json("data-file-xrefs", xref_strings)

# ---------- Markdown summary ----------
md_path = out("summary.md")
with open(md_path, "w") as f:
    f.write("# Wow.exe — first-pass artifact summary\n\n")
    f.write("Generated by `FindWagesArtifacts.py` against `%s`.\n\n" % prog_name)
    f.write("- **%d defined strings**\n" % len(strings_all))
    f.write("- **%d functions**\n" % len(funcs_all))
    f.write("- **%d external DLL imports across %d DLLs**\n" % (
        sum(len(v) for v in imports_by_dll.values()), len(imports_by_dll),
    ))
    f.write("- **%d data-file-related strings with xrefs**\n\n" % len(xref_strings))

    f.write("## DLL imports (sorted)\n\n")
    for dll in sorted(imports_by_dll.keys()):
        funcs = imports_by_dll[dll]
        f.write("### `%s` (%d funcs)\n\n" % (dll, len(funcs)))
        for fn in sorted(funcs, key=lambda x: x["name"]):
            f.write("- `%s`\n" % fn["name"])
        f.write("\n")

    f.write("## Top data-file-related strings (most-referenced first)\n\n")
    f.write("| String | Categories | Xref count | Calling functions |\n")
    f.write("|--------|------------|------------|-------------------|\n")
    for s in xref_strings[:80]:
        funcs = sorted(set(r["from_func"] for r in s["xrefs"]))
        f.write("| `%s` | %s | %d | %s |\n" % (
            s["value"].replace("|", "\\|").replace("\n", "\\n")[:80],
            ",".join(s["categories"]),
            s["xref_count"],
            ", ".join(funcs)[:120],
        ))

print("  wrote %s" % md_path)
print("FindWagesArtifacts: done")
