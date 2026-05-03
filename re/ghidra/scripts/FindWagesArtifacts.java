// FindWagesArtifacts.java — Ghidra headless script (Java)
//
// First-pass investigation of Wow.exe (Wages of War, 1996, MSVC 4.x x86).
// Exports observational data we use to ground-truth the clean-room engine.
// OUTPUT IS OBSERVATIONAL FACTS ABOUT THE BINARY — never copy of vendor source.
// Output goes to <project_dir>/../analysis/ and never into crates/.
//
//@category wages-of-war

import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.data.DataType;
import ghidra.program.model.listing.Data;
import ghidra.program.model.listing.Function;
import ghidra.program.model.listing.FunctionManager;
import ghidra.program.model.listing.Listing;
import ghidra.program.model.listing.Program;
import ghidra.program.model.symbol.ExternalLocation;
import ghidra.program.model.symbol.ExternalLocationIterator;
import ghidra.program.model.symbol.ExternalManager;
import ghidra.program.model.symbol.Reference;
import ghidra.program.model.symbol.ReferenceManager;

import java.io.File;
import java.io.FileWriter;
import java.io.IOException;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collections;
import java.util.Comparator;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.TreeMap;
import java.util.TreeSet;
import java.util.regex.Pattern;

public class FindWagesArtifacts extends GhidraScript {

    @Override
    public void run() throws Exception {
        Program program = currentProgram;
        if (program == null) {
            println("FindWagesArtifacts: no program loaded");
            return;
        }
        String progName = program.getName();
        Listing listing = program.getListing();
        FunctionManager fm = program.getFunctionManager();
        ReferenceManager refMgr = program.getReferenceManager();
        ExternalManager extMgr = program.getExternalManager();

        File projectDir = program.getDomainFile()
            .getParent().getProjectLocator().getProjectDir();
        File analysisDir = new File(projectDir.getParentFile(), "analysis");
        if (!analysisDir.isDirectory()) {
            analysisDir.mkdirs();
        }

        // ---------- Strings ----------
        println("Collecting strings...");
        List<Map<String,Object>> stringsAll = new ArrayList<>();
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
            Map<String,Object> m = new HashMap<>();
            m.put("addr", "0x" + d.getAddress().toString());
            m.put("addr_offset", d.getAddress().getOffset());
            m.put("type", dt.getName());
            m.put("len", d.getLength());
            m.put("value", s);
            stringsAll.add(m);
        }
        println("  " + stringsAll.size() + " defined strings");

        // ---------- Functions ----------
        println("Collecting functions...");
        List<Map<String,Object>> funcsAll = new ArrayList<>();
        for (Function f : fm.getFunctions(true)) {
            Address entry = f.getEntryPoint();
            long bodySize = 0;
            try {
                bodySize = f.getBody().getNumAddresses();
            } catch (Exception ignored) {}
            Map<String,Object> m = new HashMap<>();
            m.put("name", f.getName());
            m.put("entry", "0x" + entry.toString());
            m.put("entry_offset", entry.getOffset());
            m.put("size_addrs", bodySize);
            m.put("is_thunk", f.isThunk());
            m.put("is_external", f.isExternal());
            String cc = f.getCallingConventionName();
            m.put("calling_convention", cc == null ? "" : cc);
            m.put("signature", f.getSignature().toString());
            funcsAll.add(m);
        }
        println("  " + funcsAll.size() + " functions");

        // ---------- Imports (DLL functions) ----------
        println("Collecting imports...");
        Map<String, List<Map<String,Object>>> importsByDll = new TreeMap<>();
        String[] libs = extMgr.getExternalLibraryNames();
        int totalImports = 0;
        for (String lib : libs) {
            ExternalLocationIterator it = extMgr.getExternalLocations(lib);
            List<Map<String,Object>> funcs = new ArrayList<>();
            while (it.hasNext()) {
                ExternalLocation loc = it.next();
                Map<String,Object> m = new HashMap<>();
                m.put("name", loc.getLabel());
                Address a = loc.getAddress();
                m.put("addr", a == null ? null : "0x" + a.toString());
                funcs.add(m);
                totalImports++;
            }
            funcs.sort(Comparator.comparing(x -> ((String)x.get("name")).toLowerCase()));
            importsByDll.put(lib, funcs);
        }
        println("  " + libs.length + " DLLs, " + totalImports + " total imports");

        // ---------- Data-file xref strings ----------
        Object[][] patternData = {
            {"TIL", "(?i)\\.TIL\\b|TIL\\d|TILSCN"},
            {"OBJ", "(?i)\\.OBJ\\b|OBJ\\d"},
            {"MAP", "(?i)\\.MAP\\b|SCEN.*MAP|MAPS[\\\\/]"},
            {"DAT", "(?i)\\.DAT\\b|TILES\\d|JUNGSLD|RIFLWALK"},
            {"PCX", "(?i)\\.PCX\\b|OFFPIC|SCENPIC"},
            {"PAL", "(?i)\\.PAL\\b|PALETTE"},
            {"WAV", "(?i)\\.WAV\\b"},
            {"MID", "(?i)\\.MID\\b"},
            {"COR", "(?i)\\.COR\\b"},
            {"VLS", "(?i)\\.VLS\\b|\\.VLA\\b"},
            {"WRI", "(?i)\\.WRI\\b|MISHN|MSSN"},
            {"BTN", "(?i)\\.BTN\\b|BUTTONS"},
            {"CONTRACT", "(?i)CONTRACT|HIRE|MERC"},
            {"FORMAT_PRINTF", "%[-+ 0#]?[0-9]*[hl]?[a-zA-Z]"},
        };
        List<String[]> patterns = new ArrayList<>();
        List<Pattern> compiledPatterns = new ArrayList<>();
        for (Object[] row : patternData) {
            patterns.add(new String[]{(String)row[0], (String)row[1]});
            compiledPatterns.add(Pattern.compile((String)row[1]));
        }

        List<Map<String,Object>> xrefStrings = new ArrayList<>();
        for (Map<String,Object> s : stringsAll) {
            String val = (String) s.get("value");
            List<String> matchedCats = new ArrayList<>();
            for (int i = 0; i < patterns.size(); i++) {
                if (compiledPatterns.get(i).matcher(val).find()) {
                    matchedCats.add(patterns.get(i)[0]);
                }
            }
            if (matchedCats.isEmpty()) continue;
            long off = (long) s.get("addr_offset");
            Address addr = toAddr(off);
            List<Map<String,Object>> refs = new ArrayList<>();
            for (Reference r : refMgr.getReferencesTo(addr)) {
                Address fromAddr = r.getFromAddress();
                Function fn = fm.getFunctionContaining(fromAddr);
                Map<String,Object> rm = new HashMap<>();
                rm.put("from", "0x" + fromAddr.toString());
                rm.put("from_offset", fromAddr.getOffset());
                rm.put("from_func", fn == null ? "<no_func>" : fn.getName());
                rm.put("ref_type", r.getReferenceType().getName());
                refs.add(rm);
            }
            Map<String,Object> m = new HashMap<>();
            m.put("addr", s.get("addr"));
            m.put("addr_offset", off);
            m.put("value", val);
            m.put("categories", matchedCats);
            m.put("xref_count", refs.size());
            m.put("xrefs", refs);
            xrefStrings.add(m);
        }
        xrefStrings.sort((a, b) -> Integer.compare((int) b.get("xref_count"), (int) a.get("xref_count")));
        println("  " + xrefStrings.size() + " data-file-related strings with xrefs");

        // ---------- Write JSON outputs (hand-written; no Gson dep) ----------
        writeJson(new File(analysisDir, progName + "-strings.json"), stringsAll);
        writeJson(new File(analysisDir, progName + "-functions.json"), funcsAll);
        writeJson(new File(analysisDir, progName + "-imports.json"), importsByDll);
        writeJson(new File(analysisDir, progName + "-data-file-xrefs.json"), xrefStrings);

        // ---------- Markdown summary ----------
        File mdPath = new File(analysisDir, progName + "-summary.md");
        try (FileWriter fw = new FileWriter(mdPath)) {
            fw.write("# Wow.exe — first-pass artifact summary\n\n");
            fw.write("Generated by `FindWagesArtifacts.java` against `" + progName + "`.\n\n");
            fw.write("- **" + stringsAll.size() + " defined strings**\n");
            fw.write("- **" + funcsAll.size() + " functions**\n");
            fw.write("- **" + totalImports + " external DLL imports across " + libs.length + " DLLs**\n");
            fw.write("- **" + xrefStrings.size() + " data-file-related strings with xrefs**\n\n");

            fw.write("## DLL imports (sorted)\n\n");
            for (Map.Entry<String, List<Map<String,Object>>> e : importsByDll.entrySet()) {
                fw.write("### `" + e.getKey() + "` (" + e.getValue().size() + " funcs)\n\n");
                for (Map<String,Object> fn : e.getValue()) {
                    fw.write("- `" + fn.get("name") + "`\n");
                }
                fw.write("\n");
            }

            fw.write("## Top data-file-related strings (most-referenced first)\n\n");
            fw.write("| String | Categories | Xref count | Calling functions |\n");
            fw.write("|--------|------------|------------|-------------------|\n");
            int limit = Math.min(80, xrefStrings.size());
            for (int i = 0; i < limit; i++) {
                Map<String,Object> s = xrefStrings.get(i);
                @SuppressWarnings("unchecked")
                List<Map<String,Object>> refs = (List<Map<String,Object>>) s.get("xrefs");
                Set<String> funcSet = new TreeSet<>();
                for (Map<String,Object> r : refs) funcSet.add((String) r.get("from_func"));
                String val = ((String) s.get("value")).replace("|", "\\|").replace("\n", "\\n");
                if (val.length() > 80) val = val.substring(0, 80);
                @SuppressWarnings("unchecked")
                List<String> cats = (List<String>) s.get("categories");
                String funcsStr = String.join(", ", funcSet);
                if (funcsStr.length() > 120) funcsStr = funcsStr.substring(0, 120);
                fw.write("| `" + val + "` | " + String.join(",", cats)
                    + " | " + s.get("xref_count") + " | " + funcsStr + " |\n");
            }
        }
        println("  wrote " + mdPath.getAbsolutePath());
        println("FindWagesArtifacts: done");
    }

    // ----- naive JSON writer (good enough for our shapes; no quoting of keys
    // beyond replace; no nested escapes needed beyond \\ and \") -----
    private void writeJson(File path, Object obj) throws IOException {
        try (FileWriter fw = new FileWriter(path)) {
            fw.write(jsonOf(obj, 0));
            fw.write("\n");
        }
        println("  wrote " + path.getAbsolutePath());
    }

    private String jsonOf(Object o, int indent) {
        if (o == null) return "null";
        if (o instanceof Boolean) return o.toString();
        if (o instanceof Number) return o.toString();
        if (o instanceof String) return jsonString((String) o);
        if (o instanceof List) {
            List<?> list = (List<?>) o;
            if (list.isEmpty()) return "[]";
            StringBuilder sb = new StringBuilder();
            String pad = "  ".repeat(indent + 1);
            String padClose = "  ".repeat(indent);
            sb.append("[\n");
            for (int i = 0; i < list.size(); i++) {
                sb.append(pad).append(jsonOf(list.get(i), indent + 1));
                if (i < list.size() - 1) sb.append(",");
                sb.append("\n");
            }
            sb.append(padClose).append("]");
            return sb.toString();
        }
        if (o instanceof Map) {
            Map<?,?> map = (Map<?,?>) o;
            if (map.isEmpty()) return "{}";
            StringBuilder sb = new StringBuilder();
            String pad = "  ".repeat(indent + 1);
            String padClose = "  ".repeat(indent);
            sb.append("{\n");
            int i = 0;
            int n = map.size();
            for (Map.Entry<?,?> e : map.entrySet()) {
                sb.append(pad)
                  .append(jsonString(e.getKey().toString()))
                  .append(": ")
                  .append(jsonOf(e.getValue(), indent + 1));
                if (i < n - 1) sb.append(",");
                sb.append("\n");
                i++;
            }
            sb.append(padClose).append("}");
            return sb.toString();
        }
        return jsonString(o.toString());
    }

    private String jsonString(String s) {
        StringBuilder sb = new StringBuilder(s.length() + 2);
        sb.append('"');
        for (int i = 0; i < s.length(); i++) {
            char c = s.charAt(i);
            switch (c) {
                case '\\': sb.append("\\\\"); break;
                case '"':  sb.append("\\\""); break;
                case '\n': sb.append("\\n"); break;
                case '\r': sb.append("\\r"); break;
                case '\t': sb.append("\\t"); break;
                case '\b': sb.append("\\b"); break;
                case '\f': sb.append("\\f"); break;
                default:
                    if (c < 0x20) sb.append(String.format("\\u%04x", (int) c));
                    else sb.append(c);
            }
        }
        sb.append('"');
        return sb.toString();
    }
}
