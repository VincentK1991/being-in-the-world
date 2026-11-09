import com.google.gson.*;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.util.*;
import org.apache.jena.graph.*;
import org.apache.jena.rdf.model.*;
import org.apache.jena.riot.*;
import org.apache.jena.riot.lang.LabelToNode;
import org.apache.jena.shacl.*;

/** SHACL Core over explicit, local snapshots. No fetching or inference. */
public class ShaclWorker {
    static final String SH = "http://www.w3.org/ns/shacl#";
    static Property p(String name) { return ResourceFactory.createProperty(SH + name); }
    static void require(boolean value, String message) { RuleWorker.require(value, message); }
    static Model load(String text) {
        require(text.getBytes(StandardCharsets.UTF_8).length <= 16*1024*1024, "snapshot exceeds 16 MiB");
        Model model = ModelFactory.createDefaultModel();
        RDFParser.fromString(text, Lang.NTRIPLES).labelToNode(LabelToNode.createUseLabelAsGiven()).parse(model);
        require(model.size() <= 100000, "snapshot exceeds 100000 triples");
        return model;
    }
    static JsonArray terms(Resource result, String name) {
        JsonArray values = new JsonArray();
        var it = result.listProperties(p(name));
        try { while(it.hasNext()) values.add(RuleWorker.format(it.next().getObject().asNode())); }
        finally { it.close(); }
        return values;
    }
    static JsonObject validate(JsonObject input) {
        Model data = load(input.get("data").getAsString());
        Model shapes = load(input.get("shapes").getAsString());
        try {
            // This release deliberately exposes Core only. Reject extension declarations rather than silently ignore them.
            for(String name : List.of("sparql", "select", "ask", "js", "jsFunctionName", "jsLibrary", "target", "rule", "entailment", "validator", "nodeValidator", "propertyValidator"))
                require(!shapes.contains(null, p(name)), "only SHACL Core is supported; disabled sh:" + name);
            require(!shapes.contains(null, ResourceFactory.createProperty("http://www.w3.org/2002/07/owl#imports")), "shape imports are disabled");
            Shapes parsed = Shapes.parse(shapes.getGraph());
            require(parsed.numShapes() > 0, "shapes graph has no shapes");
            ValidationReport report = ShaclValidator.get().validate(parsed, data.getGraph());
            JsonArray findings = new JsonArray();
            var results = report.getResource().listProperties(p("result"));
            try {
                while(results.hasNext()) {
                    require(findings.size() < 1000, "report exceeds 1000 results; no partial report published");
                    Resource result = results.next().getResource();
                    JsonObject finding = new JsonObject();
                    for(String name : List.of("focusNode", "resultPath", "value", "sourceShape", "sourceConstraintComponent", "resultMessage"))
                        finding.add(name, terms(result, name));
                    String severity = result.getRequiredProperty(p("resultSeverity")).getResource().getURI();
                    require(List.of(SH+"Violation", SH+"Warning", SH+"Info").contains(severity), "unsupported custom SHACL severity");
                    finding.addProperty("severity", severity.substring(SH.length()));
                    findings.add(finding);
                }
            } finally { results.close(); }
            StringBuilder nt = new StringBuilder();
            var triples = report.getModel().getGraph().find();
            try {
                while(triples.hasNext()) {
                    Triple t = triples.next();
                    nt.append(RuleWorker.format(t.getSubject())).append(' ').append(RuleWorker.format(t.getPredicate())).append(' ').append(RuleWorker.format(t.getObject())).append(" .\n");
                    require(nt.length() <= 4*1024*1024, "report exceeds character limit");
                }
            } finally { triples.close(); }
            JsonObject out = new JsonObject();
            out.addProperty("conforms", report.conforms());
            out.add("findings", findings);
            out.addProperty("ntriples", nt.toString());
            out.addProperty("engine_version", org.apache.jena.Jena.VERSION);
            return out;
        } finally { data.close(); shapes.close(); }
    }
    public static void main(String[] args) {
        try { System.out.println(validate(JsonParser.parseReader(new InputStreamReader(System.in, StandardCharsets.UTF_8)).getAsJsonObject())); }
        catch(Exception error) { JsonObject out = new JsonObject(); out.addProperty("error", error.getMessage()); System.out.println(out); System.exit(1); }
    }
}
