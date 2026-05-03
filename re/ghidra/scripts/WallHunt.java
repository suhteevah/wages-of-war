// WallHunt.java — find every function that touches wall/fence/edge data.
//
// Strategy: locate every defined string whose value contains "wall", "fence",
// "edge", "side", "Pass", "PASS", "WALL", or "FENCE", then decompile every
// function with a reference to one of those addresses. The "Can't Pass Wall"
// message is what we're really after — whoever emits it has the wall data
// in scope and tells us the encoding.
//
//@category wages-of-war

import ghidra.app.decompiler.DecompInterface;
import ghidra.app.decompiler.DecompileResults;
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.data.DataType;
import ghidra.program.model.listing.Data;
import ghidra.program.model.listing.Function;
import ghidra.program.model.listing.FunctionManager;
import ghidra.program.model.listing.Listing;
import ghidra.program.model.listing.Program;
import ghidra.program.model.symbol.Reference;
import ghidra.program.model.symbol.ReferenceManager;

import java.io.File;
import java.io.FileWriter;
import java.util.ArrayList;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Set;
import java.util.regex.Pattern;

public class WallHunt extends GhidraScript {

    @Override
    public void run() throws Exception {
        Program program = currentProgram;
        if (program == null) { println("no program"); return; }

        File projectDir = program.getDomainFile()
            .getParent().getProjectLocator().getProjectDir();
        File analysisDir = new File(projectDir.getParentFile(), "analysis");
        File decompDir = new File(analysisDir, "decomp");
        if (!decompDir.isDirectory()) decompDir.mkdirs();

        Listing listing = program.getListing();
        FunctionManager fm = program.getFunctionManager();
        ReferenceManager refMgr = program.getReferenceManager();

        Pattern wallRx = Pattern.compile("(?i)wall|fence|edge|pass|side|movecost|impass|blocked|barrier");

        // Step 1: collect all strings matching the pattern.
        List<Data> matchedStrings = new ArrayList<>();
        var dataIter = listing.getDefinedData(true);
        while (dataIter.hasNext()) {
            Data d = dataIter.next();
            DataType dt = d.getDataType();
            if (dt == null) continue;
            String tname = dt.getName().toLowerCase();
            if (!(tname.contains("string") || tname.startsWith("char[") || tname.equals("char"))) continue;
            Object v = d.getValue();
            if (v == null) continue;
            String s = v.toString();
            if (s.length() < 3) continue;
            if (wallRx.matcher(s).find()) {
                matchedStrings.add(d);
            }
        }
        println("Matched " + matchedStrings.size() + " wall/fence-y strings");

        // Step 2: gather containing functions.
        Set<Function> targets = new LinkedHashSet<>();
        StringBuilder index = new StringBuilder();
        index.append("# Wall/fence-related strings + callers\n\n");
        index.append("| String | Addr | Callers |\n|---|---|---|\n");
        for (Data d : matchedStrings) {
            String val = d.getValue().toString().replace("|", "\\|").replace("\n", "\\n");
            if (val.length() > 60) val = val.substring(0, 60);
            Set<String> callers = new LinkedHashSet<>();
            for (Reference r : refMgr.getReferencesTo(d.getAddress())) {
                Function f = fm.getFunctionContaining(r.getFromAddress());
                if (f != null) {
                    callers.add(f.getName());
                    targets.add(f);
                }
            }
            index.append("| `").append(val).append("` | 0x").append(d.getAddress()).append(" | ")
                 .append(String.join(", ", callers)).append(" |\n");
        }
        println("Total callers to decompile: " + targets.size());

        File idxFile = new File(decompDir, "_wallhunt_index.md");
        try (FileWriter fw = new FileWriter(idxFile)) { fw.write(index.toString()); }

        // Step 3: decompile each.
        DecompInterface decomp = new DecompInterface();
        if (!decomp.openProgram(program)) { println("decomp open failed"); return; }
        try {
            int n = 0;
            for (Function f : targets) {
                String name = f.getName();
                File outFile = new File(decompDir, "wallhunt_" + name + ".c");
                if (outFile.exists()) {
                    println("skip (already exists): " + name);
                    continue;
                }
                println("Decompiling " + name + " @ " + f.getEntryPoint());
                DecompileResults res = decomp.decompileFunction(f, 60, monitor);
                String body;
                if (res != null && res.decompileCompleted()) {
                    var out = res.getDecompiledFunction();
                    body = (out != null) ? out.getC() : "// decompiler returned no body\n";
                } else {
                    body = "// decompile failed: "
                        + (res == null ? "null" : res.getErrorMessage()) + "\n";
                }
                try (FileWriter fw = new FileWriter(outFile)) {
                    fw.write("// " + name + " @ " + f.getEntryPoint() + "\n");
                    fw.write("// CLEAN-ROOM: observational. Do not paste into crates/.\n\n");
                    fw.write(body);
                }
                n++;
            }
            println("WallHunt: decompiled " + n + " new functions");
        } finally {
            decomp.dispose();
        }
    }
}
