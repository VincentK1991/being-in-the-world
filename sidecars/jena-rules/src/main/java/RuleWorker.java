import com.google.gson.*;
import java.util.*;
import java.nio.charset.StandardCharsets;
import org.apache.jena.graph.Node;
import org.apache.jena.rdf.model.*;
import org.apache.jena.reasoner.TriplePattern;
import org.apache.jena.reasoner.rulesys.*;
import org.apache.jena.riot.*;
import org.apache.jena.riot.lang.LabelToNode;
import org.apache.jena.riot.out.NodeFmtLib;

public class RuleWorker {
    static void require(boolean value, String message) { if (!value) throw new IllegalArgumentException(message); }
    static Node[] terms(ClauseEntry clause) {
        require(clause instanceof TriplePattern, "only triple-pattern clauses are supported; built-ins and nested rules are disabled");
        TriplePattern triple=(TriplePattern)clause;
        Node[] terms={triple.getSubject(),triple.getPredicate(),triple.getObject()};
        for(Node node:terms) require(node.isVariable() || node.isURI() || node.isLiteral(), "blank-node constants, wildcards and functors are disabled");
        require(!Functor.isFunctor(triple.getObject()),"functors are disabled");
        require(!triple.getSubject().isLiteral() && !triple.getPredicate().isLiteral(),"invalid RDF triple pattern");
        return terms;
    }
    static String format(Node node) {
        // Labels originate from the trusted dataset snapshot and must remain joinable.
        return node.isBlank() ? "_:"+node.getBlankNodeLabel() : NodeFmtLib.strNT(node);
    }
    static JsonObject infer(JsonObject input) {
        require(input.get("profile").getAsString().equals("jena-forward-v1"),"unsupported rule profile");
        JsonArray documents=input.getAsJsonArray("rules");
        require(documents!=null && documents.size()>0 && documents.size()<=64,"provide 1 to 64 rule documents");
        List<Rule> rules=new ArrayList<>(); int bytes=0;
        for(JsonElement document:documents) {
            String text=document.getAsString(); bytes+=text.getBytes(StandardCharsets.UTF_8).length;
            require(bytes<=1024*1024,"rules exceed 1 MiB");
            require(!java.util.regex.Pattern.compile("(?i)@\\s*include").matcher(text).find(),"rule imports disabled");
            rules.addAll(Rule.parseRules(text));
        }
        require(!rules.isEmpty() && rules.size()<=256,"provide 1 to 256 rules");
        for(Rule rule:rules) {
            require(!rule.isBackward() && rule.bodyLength()>0 && rule.headLength()>0,"only forward rules with nonempty body/head are supported");
            Set<Node> bound=new HashSet<>();
            for(ClauseEntry clause:rule.getBody()) for(Node node:terms(clause)) if(node.isVariable()) bound.add(node);
            for(ClauseEntry clause:rule.getHead()) for(Node node:terms(clause)) require(!node.isVariable() || bound.contains(node),"head variable is not bound by a body triple");
        }
        String text=input.get("ntriples").getAsString();
        require(text.getBytes(StandardCharsets.UTF_8).length<=16*1024*1024,"source exceeds 16 MiB");
        Model base=ModelFactory.createDefaultModel();
        RDFParser.fromString(text, Lang.NTRIPLES).labelToNode(LabelToNode.createUseLabelAsGiven()).parse(base);
        require(base.size()<=100000,"source exceeds 100000 triples");
        GenericRuleReasoner reasoner=new GenericRuleReasoner(rules);
        reasoner.setMode(GenericRuleReasoner.FORWARD_RETE);
        InfModel inferred=ModelFactory.createInfModel(reasoner,base);
        inferred.prepare();
        TreeSet<String> lines=new TreeSet<>(); int outputBytes=0;
        var it=inferred.getDeductionsModel().getGraph().find();
        try {
            while(it.hasNext()) {
                var triple=it.next(); if(base.getGraph().contains(triple)) continue;
                String line=format(triple.getSubject())+" "+format(triple.getPredicate())+" "+format(triple.getObject())+" .\n";
                if(lines.add(line)) outputBytes+=line.getBytes(StandardCharsets.UTF_8).length;
                require(lines.size()<=200000 && outputBytes<=16*1024*1024,"derived output exceeds limit");
            }
        } finally { it.close(); inferred.close(); base.close(); }
        JsonObject result=new JsonObject(); result.addProperty("ntriples",String.join("",lines));
        result.addProperty("engine_version",org.apache.jena.Jena.VERSION);
        return result;
    }
    public static void main(String[] args) {
        try { System.out.println(infer(JsonParser.parseReader(new java.io.InputStreamReader(System.in,StandardCharsets.UTF_8)).getAsJsonObject())); }
        catch(Exception error) {JsonObject result=new JsonObject();result.addProperty("error",error.getMessage());System.out.println(result);System.exit(1);}
    }
}
