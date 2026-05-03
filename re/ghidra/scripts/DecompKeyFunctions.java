// DecompKeyFunctions.java — Ghidra headless script
//
// Decompile a curated set of high-value functions in Wow.exe and write the
// pseudo-C to disk. These functions were identified by FindWagesArtifacts.java:
//
//   FUN_00429fc6  — references "Tile = %d, Grid = %d" + "lumpy.cor".
//                   Almost certainly the per-tile render dispatch.
//   FUN_004342ca  — references "Overwriting Memory. action=%d spriteid=%d
//                   size=%ld". Sprite system bounds-check site.
//   FUN_004075ea  — opens mis01.obj … mis15.obj. Mission-OBJ loader.
//   FUN_00405d47  — phonspr.obj / mom.obj / pizza.obj / shark.obj. Debrief
//                   intro/cutscene loader.
//   FUN_0043f850  — calcspr.obj. Sprite computation.
//   FUN_00417a79  — armexc.obj + mantoman.btn. Equipment + combat-buttons.
//
// We also walk every callsite of WinGBitBlt / WinGCreateBitmap / WinGCreateDC
// and decompile the *containing* function — that gives us the renderer entry
// without having to guess at addresses.
//
// Output:
//   <project_dir>/../analysis/decomp/<func_name>.c
//   <project_dir>/../analysis/decomp/_index.md
//
//@category wages-of-war

import ghidra.app.decompiler.DecompInterface;
import ghidra.app.decompiler.DecompileResults;
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.listing.Function;
import ghidra.program.model.listing.FunctionManager;
import ghidra.program.model.listing.Program;
import ghidra.program.model.symbol.ExternalLocation;
import ghidra.program.model.symbol.ExternalLocationIterator;
import ghidra.program.model.symbol.ExternalManager;
import ghidra.program.model.symbol.Reference;
import ghidra.program.model.symbol.ReferenceManager;
import ghidra.program.model.symbol.Symbol;
import ghidra.program.model.symbol.SymbolTable;

import java.io.File;
import java.io.FileWriter;
import java.io.IOException;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.TreeSet;

public class DecompKeyFunctions extends GhidraScript {

    @Override
    public void run() throws Exception {
        Program program = currentProgram;
        if (program == null) {
            println("DecompKeyFunctions: no program loaded");
            return;
        }

        File projectDir = program.getDomainFile()
            .getParent().getProjectLocator().getProjectDir();
        File analysisDir = new File(projectDir.getParentFile(), "analysis");
        File decompDir = new File(analysisDir, "decomp");
        if (!decompDir.isDirectory()) decompDir.mkdirs();

        FunctionManager fm = program.getFunctionManager();
        SymbolTable st = program.getSymbolTable();
        ReferenceManager refMgr = program.getReferenceManager();
        ExternalManager extMgr = program.getExternalManager();

        // ---------- Curated targets by name ----------
        String[] hardTargets = {
            "FUN_00429fc6",
            "FUN_004342ca",
            "FUN_004075ea",
            "FUN_00405d47",
            "FUN_0043f850",
            "FUN_00417a79",
        };

        // ---------- Targets by import xref ----------
        String[] importsToFollow = {
            "WinGBitBlt",
            "WinGCreateBitmap",
            "WinGCreateDC",
            "WinGSetDIBColorTable",
            "WinGRecommendDIBFormat",
            // Also the file-open dialog: whoever consumes the editor filter
            // string is likely the editor entry point.
            "GetOpenFileNameA",
            // Palette plumbing — the function that sets the WoW palette.
            "RealizePalette",
            "AnimatePalette",
        };

        // Resolve hard targets to Functions, ordered.
        Set<Function> targetSet = new java.util.LinkedHashSet<>();

        for (String name : hardTargets) {
            Function f = findFunctionByName(fm, st, name);
            if (f != null) {
                targetSet.add(f);
                println("Resolved hard target: " + name + " @ " + f.getEntryPoint());
            } else {
                println("WARN: could not resolve hard target: " + name);
            }
        }

        // For each import, find every reference to its address and resolve
        // the containing function. Add to target set.
        for (String impName : importsToFollow) {
            int hits = 0;
            for (String dll : extMgr.getExternalLibraryNames()) {
                ExternalLocationIterator it = extMgr.getExternalLocations(dll);
                while (it.hasNext()) {
                    ExternalLocation loc = it.next();
                    if (!impName.equals(loc.getLabel())) continue;
                    // The thunk address — look up references TO it.
                    Symbol sym = loc.getSymbol();
                    if (sym == null) continue;
                    Address thunkAddr = sym.getAddress();

                    for (Reference r : refMgr.getReferencesTo(thunkAddr)) {
                        Function caller = fm.getFunctionContaining(r.getFromAddress());
                        if (caller != null && targetSet.add(caller)) {
                            hits++;
                        }
                    }
                }
            }
            println("Import xrefs added " + hits + " function(s) for " + impName);
        }

        println("Total functions to decompile: " + targetSet.size());

        // ---------- Decompile each ----------
        DecompInterface decomp = new DecompInterface();
        if (!decomp.openProgram(program)) {
            println("ERROR: Failed to open program in decompiler");
            return;
        }

        try (FileWriter idx = new FileWriter(new File(decompDir, "_index.md"))) {
            idx.write("# Decompiled Functions\n\n");
            idx.write("| Function | Entry | Size (addrs) | Output |\n");
            idx.write("|----------|-------|--------------|--------|\n");

            int decompiled = 0;
            for (Function f : targetSet) {
                String name = f.getName();
                Address entry = f.getEntryPoint();
                long size = 0;
                try { size = f.getBody().getNumAddresses(); } catch (Exception ignored) {}

                println("Decompiling " + name + " @ " + entry + " (" + size + " addrs) ...");
                DecompileResults res = decomp.decompileFunction(f, 60, monitor);
                String body;
                if (res != null && res.decompileCompleted()) {
                    var out = res.getDecompiledFunction();
                    body = (out != null) ? out.getC() : "// decompiler returned no body\n";
                } else {
                    body = "// decompile failed: "
                        + (res == null ? "null result" : res.getErrorMessage())
                        + "\n";
                }

                String fname = name + ".c";
                File outFile = new File(decompDir, fname);
                try (FileWriter fw = new FileWriter(outFile)) {
                    fw.write("// " + name + " @ " + entry + " (" + size + " addrs)\n");
                    fw.write("// Decompiled from Wow.exe (Wages of War, 1996, Random Games)\n");
                    fw.write("// CLEAN-ROOM: this is observational. Do not paste into crates/.\n\n");
                    fw.write(body);
                }
                idx.write("| `" + name + "` | 0x" + entry + " | " + size + " | [" + fname + "](" + fname + ") |\n");
                decompiled++;
            }
            println("Decompiled " + decompiled + " functions to " + decompDir.getAbsolutePath());
        } finally {
            decomp.dispose();
        }
    }

    private Function findFunctionByName(FunctionManager fm, SymbolTable st, String name) {
        // Symbols can have the same name across namespaces; take the first
        // function-typed match.
        for (Symbol s : st.getSymbols(name)) {
            Function f = fm.getFunctionAt(s.getAddress());
            if (f != null) return f;
        }
        // Fall back to a raw scan.
        for (Function f : fm.getFunctions(true)) {
            if (name.equals(f.getName())) return f;
        }
        return null;
    }
}
