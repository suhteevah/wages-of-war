// WallStructure.java — decompile the wall-data filler and tile neighbor
// helpers identified in FUN_0041d2c6 (GridPass). Goal: learn how walls are
// stored per tile (storage offset, byte count, layout).
//
//@category wages-of-war

import ghidra.app.decompiler.DecompInterface;
import ghidra.app.decompiler.DecompileResults;
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.listing.Function;
import ghidra.program.model.listing.FunctionManager;
import ghidra.program.model.listing.Program;
import ghidra.program.model.symbol.Symbol;
import ghidra.program.model.symbol.SymbolTable;

import java.io.File;
import java.io.FileWriter;
import java.util.LinkedHashSet;
import java.util.Set;

public class WallStructure extends GhidraScript {

    @Override
    public void run() throws Exception {
        Program program = currentProgram;
        if (program == null) return;

        File projectDir = program.getDomainFile()
            .getParent().getProjectLocator().getProjectDir();
        File analysisDir = new File(projectDir.getParentFile(), "analysis");
        File decompDir = new File(analysisDir, "decomp");
        if (!decompDir.isDirectory()) decompDir.mkdirs();

        String[] targets = {
            "FUN_0041b26d", // wall-data filler — fills DAT_005ee075..0x80
            "FUN_0041c81c", // direction translator (grid, dir) -> wall slot
            "FUN_0041bdeb", // tile-neighbor helper A
            "FUN_0041bc69", // tile-neighbor helper B
            "FUN_0041bf6d", // tile-neighbor helper C
            "FUN_0041bae7", // tile-neighbor helper D
            "FUN_0041c02a", // grid translator (called early in GridPass)
            "FUN_0045fe11", // screen-coords -> tile/grid lookup
        };

        FunctionManager fm = program.getFunctionManager();
        SymbolTable st = program.getSymbolTable();

        Set<Function> set = new LinkedHashSet<>();
        for (String name : targets) {
            for (Symbol s : st.getSymbols(name)) {
                Function f = fm.getFunctionAt(s.getAddress());
                if (f != null) { set.add(f); break; }
            }
        }
        println("Resolved " + set.size() + " of " + targets.length);

        DecompInterface decomp = new DecompInterface();
        if (!decomp.openProgram(program)) return;
        try {
            for (Function f : set) {
                String name = f.getName();
                File out = new File(decompDir, "wallstruct_" + name + ".c");
                if (out.exists()) { println("skip " + name); continue; }
                println("Decompiling " + name + " @ " + f.getEntryPoint());
                DecompileResults res = decomp.decompileFunction(f, 60, monitor);
                String body = (res != null && res.decompileCompleted())
                    ? res.getDecompiledFunction().getC()
                    : "// decompile failed\n";
                try (FileWriter fw = new FileWriter(out)) {
                    fw.write("// " + name + " @ " + f.getEntryPoint() + "\n");
                    fw.write("// CLEAN-ROOM observational\n\n");
                    fw.write(body);
                }
            }
        } finally {
            decomp.dispose();
        }
        println("WallStructure: done");
    }
}
