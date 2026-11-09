//! OWL 2 RL/RDF rules (W3C OWL 2 Profiles, section 4.3) evaluated to a fixpoint in memory.
//!
//! Tables 4 (equality), 5 (properties), 6 (classes), 7 (class axioms) and 9 (schema) are
//! implemented; table 8 (datatypes) is not. The first justification of every derived triple is
//! kept (the rule and the triples it matched), so any deduction can be explained back to the
//! input. Output follows the `owlrl-full-v1` policy: nothing about RDF/RDFS/OWL/XSD terms the input
//! never mentions, and nothing that holds of every resource (`x a owl:Thing`).
use anyhow::{Context, Result, ensure};
use rdf_plugin_inference::Inconsistent;
use rdf_store::comparison::by_value;
use rdf_store::oxigraph::{
    io::{RdfFormat, RdfParser},
    model::{NamedNode, Term, Triple},
};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const RDFS: &str = "http://www.w3.org/2000/01/rdf-schema#";
const OWL: &str = "http://www.w3.org/2002/07/owl#";
const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
const MAX_INPUT: usize = 100_000;
const MAX_CHAIN_PATHS: usize = 100_000;
const MAX_CLASHES: usize = 100;
const MAX_PROOF_NODES: usize = 200;
const MAX_PROOF_DEPTH: usize = 12;

type Id = u32;
type T = (Id, Id, Id);

/// Why a triple holds: the rule and the triples it matched.
#[derive(Clone)]
struct Why {
    rule: &'static str,
    premises: Vec<T>,
}
struct Clash {
    rule: &'static str,
    message: String,
    premises: Vec<T>,
}

#[derive(Default)]
struct Terms {
    ids: HashMap<Term, Id>,
    terms: Vec<Term>,
}
impl Terms {
    fn id(&mut self, term: Term) -> Id {
        if let Some(id) = self.ids.get(&term) {
            return *id;
        }
        let id = self.terms.len() as Id;
        self.ids.insert(term.clone(), id);
        self.terms.push(term);
        id
    }
    fn iri(&mut self, iri: &str) -> Id {
        self.id(NamedNode::new_unchecked(iri).into())
    }
    fn term(&self, id: Id) -> &Term {
        &self.terms[id as usize]
    }
    fn literal(&self, id: Id) -> bool {
        matches!(self.term(id), Term::Literal(_))
    }
    /// The non-negative integer a cardinality literal states, if it is one.
    fn number(&self, id: Id) -> Option<u64> {
        match self.term(id) {
            Term::Literal(l) if l.datatype().as_str() == format!("{XSD}nonNegativeInteger") => {
                l.value().trim().parse().ok()
            }
            _ => None,
        }
    }
    fn line(&self, (s, p, o): T) -> String {
        format!("{} {} {} .", self.term(s), self.term(p), self.term(o))
    }
}

#[derive(Default)]
struct Graph {
    set: HashSet<T>,
    sp: HashMap<(Id, Id), Vec<Id>>,
    po: HashMap<(Id, Id), Vec<Id>>,
    p: HashMap<Id, Vec<(Id, Id)>>,
    s: HashMap<Id, Vec<(Id, Id)>>,
    o: HashMap<Id, Vec<(Id, Id)>>,
}
impl Graph {
    fn insert(&mut self, t: T) -> bool {
        if !self.set.insert(t) {
            return false;
        }
        let (s, p, o) = t;
        self.sp.entry((s, p)).or_default().push(o);
        self.po.entry((p, o)).or_default().push(s);
        self.p.entry(p).or_default().push((s, o));
        self.s.entry(s).or_default().push((p, o));
        self.o.entry(o).or_default().push((s, p));
        true
    }
    fn has(&self, t: T) -> bool {
        self.set.contains(&t)
    }
    fn objects(&self, s: Id, p: Id) -> &[Id] {
        self.sp.get(&(s, p)).map_or(&[], Vec::as_slice)
    }
    fn subjects(&self, p: Id, o: Id) -> &[Id] {
        self.po.get(&(p, o)).map_or(&[], Vec::as_slice)
    }
    fn pairs(&self, p: Id) -> &[(Id, Id)] {
        self.p.get(&p).map_or(&[], Vec::as_slice)
    }
    fn about(&self, s: Id) -> &[(Id, Id)] {
        self.s.get(&s).map_or(&[], Vec::as_slice)
    }
    fn mentioning(&self, o: Id) -> &[(Id, Id)] {
        self.o.get(&o).map_or(&[], Vec::as_slice)
    }
}

/// The vocabulary the rules match on.
struct V {
    ty: Id,
    sub_class: Id,
    sub_prop: Id,
    domain: Id,
    range: Id,
    same: Id,
    different: Id,
    eq_class: Id,
    eq_prop: Id,
    inverse: Id,
    first: Id,
    rest: Id,
    nil: Id,
    thing: Id,
    nothing: Id,
    class: Id,
    object_property: Id,
    datatype_property: Id,
    transitive: Id,
    symmetric: Id,
    asymmetric: Id,
    irreflexive: Id,
    functional: Id,
    inverse_functional: Id,
    prop_disjoint: Id,
    all_disjoint_props: Id,
    all_disjoint_classes: Id,
    all_different: Id,
    members: Id,
    distinct_members: Id,
    chain: Id,
    key: Id,
    intersection: Id,
    union: Id,
    complement: Id,
    some: Id,
    all: Id,
    has_value: Id,
    on_property: Id,
    on_class: Id,
    max_card: Id,
    max_qcard: Id,
    one_of: Id,
    disjoint: Id,
    npa_source: Id,
    npa_property: Id,
    npa_target: Id,
    npa_value: Id,
}
impl V {
    fn new(t: &mut Terms) -> Self {
        let (r, s, o) = (
            |n: &str| format!("{RDF}{n}"),
            |n: &str| format!("{RDFS}{n}"),
            |n: &str| format!("{OWL}{n}"),
        );
        V {
            ty: t.iri(&r("type")),
            sub_class: t.iri(&s("subClassOf")),
            sub_prop: t.iri(&s("subPropertyOf")),
            domain: t.iri(&s("domain")),
            range: t.iri(&s("range")),
            same: t.iri(&o("sameAs")),
            different: t.iri(&o("differentFrom")),
            eq_class: t.iri(&o("equivalentClass")),
            eq_prop: t.iri(&o("equivalentProperty")),
            inverse: t.iri(&o("inverseOf")),
            first: t.iri(&r("first")),
            rest: t.iri(&r("rest")),
            nil: t.iri(&r("nil")),
            thing: t.iri(&o("Thing")),
            nothing: t.iri(&o("Nothing")),
            class: t.iri(&o("Class")),
            object_property: t.iri(&o("ObjectProperty")),
            datatype_property: t.iri(&o("DatatypeProperty")),
            transitive: t.iri(&o("TransitiveProperty")),
            symmetric: t.iri(&o("SymmetricProperty")),
            asymmetric: t.iri(&o("AsymmetricProperty")),
            irreflexive: t.iri(&o("IrreflexiveProperty")),
            functional: t.iri(&o("FunctionalProperty")),
            inverse_functional: t.iri(&o("InverseFunctionalProperty")),
            prop_disjoint: t.iri(&o("propertyDisjointWith")),
            all_disjoint_props: t.iri(&o("AllDisjointProperties")),
            all_disjoint_classes: t.iri(&o("AllDisjointClasses")),
            all_different: t.iri(&o("AllDifferent")),
            members: t.iri(&o("members")),
            distinct_members: t.iri(&o("distinctMembers")),
            chain: t.iri(&o("propertyChainAxiom")),
            key: t.iri(&o("hasKey")),
            intersection: t.iri(&o("intersectionOf")),
            union: t.iri(&o("unionOf")),
            complement: t.iri(&o("complementOf")),
            some: t.iri(&o("someValuesFrom")),
            all: t.iri(&o("allValuesFrom")),
            has_value: t.iri(&o("hasValue")),
            on_property: t.iri(&o("onProperty")),
            on_class: t.iri(&o("onClass")),
            max_card: t.iri(&o("maxCardinality")),
            max_qcard: t.iri(&o("maxQualifiedCardinality")),
            one_of: t.iri(&o("oneOf")),
            disjoint: t.iri(&o("disjointWith")),
            npa_source: t.iri(&o("sourceIndividual")),
            npa_property: t.iri(&o("assertionProperty")),
            npa_target: t.iri(&o("targetIndividual")),
            npa_value: t.iri(&o("targetValue")),
        }
    }
}

/// Triples added in the last round, indexed by predicate.
struct Delta {
    all: Vec<T>,
    by_p: HashMap<Id, Vec<(Id, Id)>>,
}
impl Delta {
    fn new(all: Vec<T>) -> Self {
        let mut by_p: HashMap<Id, Vec<(Id, Id)>> = HashMap::new();
        for &(s, p, o) in &all {
            by_p.entry(p).or_default().push((s, o));
        }
        Delta { all, by_p }
    }
    fn pairs(&self, p: Id) -> &[(Id, Id)] {
        self.by_p.get(&p).map_or(&[], Vec::as_slice)
    }
}
/// One round's findings before they are added.
#[derive(Default)]
struct Round {
    new: Vec<(T, Why)>,
    /// Conclusions already proposed this round, so each is justified (and allocated) once.
    pending: HashSet<T>,
    clashes: Vec<Clash>,
    overflow: bool,
}
impl Round {
    fn add(&mut self, g: &Graph, t: T, rule: &'static str, premises: Vec<T>) {
        self.add_with(g, t, rule, || premises);
    }
    /// Like `add`, but the premises are only built for a conclusion that is actually new.
    fn add_with(&mut self, g: &Graph, t: T, rule: &'static str, premises: impl FnOnce() -> Vec<T>) {
        if !g.has(t) && !self.pending.contains(&t) {
            if self.pending.len() >= 200_000 {
                self.overflow = true;
                return;
            }
            self.pending.insert(t);
            self.new.push((t, Why { rule, premises: premises() }));
        }
    }
    fn clash(&mut self, rule: &'static str, message: String, premises: Vec<T>) {
        if self.clashes.len() >= MAX_CLASHES {
            return;
        }
        self.clashes.push(Clash { rule, message, premises });
    }
}
pub struct Closure {
    terms: Terms,
    graph: Graph,
    input: HashSet<T>,
    why: HashMap<T, Why>,
    clashes: Vec<Clash>,
    v: V,
    deadline: Instant,
}

impl Closure {
    pub fn compute(ntriples: &str, max_derived: usize, timeout: Duration) -> Result<Self> {
        let mut terms = Terms::default();
        let v = V::new(&mut terms);
        let mut graph = Graph::default();
        let mut input = HashSet::new();
        for (count, quad) in
            RdfParser::from_format(RdfFormat::NTriples).for_reader(ntriples.as_bytes()).enumerate()
        {
            ensure!(count < MAX_INPUT, "source exceeds {MAX_INPUT} triples");
            let quad = quad?;
            let t = (
                terms.id(quad.subject.into()),
                terms.id(quad.predicate.into()),
                terms.id(quad.object),
            );
            graph.insert(t);
            input.insert(t);
        }
        let mut closure = Closure {
            terms,
            graph,
            input,
            why: HashMap::new(),
            clashes: Vec::new(),
            v,
            deadline: Instant::now() + timeout,
        };
        closure.reflexive_equality();
        // Semi-naive: the hot rules only join triples new in the last round (all of them at first).
        let mut delta = Delta::new(closure.graph.set.iter().copied().collect());
        let started = Instant::now();
        loop {
            ensure!(
                started.elapsed() <= timeout,
                "reasoning exceeded {} seconds",
                timeout.as_secs()
            );
            let round = closure.round(&delta)?;
            closure.clashes.extend(round.clashes);
            if closure.clashes.len() >= MAX_CLASHES {
                break;
            }
            let mut added = Vec::new();
            for (t, why) in round.new {
                if closure.terms.literal(t.0) {
                    continue;
                }
                if closure.graph.insert(t) {
                    closure.why.insert(t, why);
                    added.push(t);
                }
            }
            ensure!(
                closure.why.len() <= max_derived,
                "derived output exceeds {max_derived} triples"
            );
            if added.is_empty() {
                break;
            }
            delta = Delta::new(added);
        }
        Ok(closure)
    }

    /// eq-ref: every resource is the same as itself. Done once: rules never introduce new terms.
    fn reflexive_equality(&mut self) {
        let mut seen = HashSet::new();
        let input: Vec<T> = self.input.iter().copied().collect();
        for t in input {
            for x in [t.0, t.1, t.2] {
                if !self.terms.literal(x) && seen.insert(x) {
                    let r = (x, self.v.same, x);
                    if self.graph.insert(r) {
                        self.why.insert(r, Why { rule: "eq-ref", premises: vec![t] });
                    }
                }
            }
        }
    }

    /// An RDF list's members, with the list triples that state them. `None` if it is malformed.
    fn list(&self, mut head: Id) -> Option<(Vec<Id>, Vec<T>)> {
        let (mut items, mut used, mut seen) = (Vec::new(), Vec::new(), HashSet::new());
        while head != self.v.nil {
            if !seen.insert(head) {
                return None;
            }
            let [first] = self.graph.objects(head, self.v.first) else { return None };
            let [rest] = self.graph.objects(head, self.v.rest) else { return None };
            let (first, rest) = (*first, *rest);
            used.push((head, self.v.first, first));
            used.push((head, self.v.rest, rest));
            items.push(first);
            head = rest;
        }
        Some((items, used))
    }
    fn typed(&self, x: Id, c: Id) -> bool {
        self.graph.has((x, self.v.ty, c))
    }
    fn instances(&self, c: Id) -> &[Id] {
        self.graph.subjects(self.v.ty, c)
    }
    fn with_type(&self, c: Id) -> Vec<Id> {
        self.instances(c).to_vec()
    }

    fn check_budget(&self) -> Result<()> {
        ensure!(Instant::now() <= self.deadline, "reasoning deadline exceeded");
        Ok(())
    }
    fn round(&self, delta: &Delta) -> Result<Round> {
        let mut r = Round::default();
        self.incremental(delta, &mut r)?;
        self.equality(&mut r)?;
        self.properties(&mut r)?;
        self.classes(&mut r)?;
        self.class_axioms(&mut r)?;
        self.schema(&mut r)?;
        self.check_budget()?;
        ensure!(!r.overflow, "reasoning round exceeds intermediate conclusion limit");
        Ok(r)
    }

    /// The rules that dominate on large inputs, evaluated semi-naively: a conclusion is only
    /// looked for where at least one premise is new since the last round. In naive evaluation,
    /// a combination of older triples was already tried a round earlier, so nothing is missed.
    /// eq-sym, eq-trans, eq-rep-s/p/o, prp-dom, prp-rng, prp-spo1, prp-trp, cax-sco, scm-sco, scm-spo.
    fn incremental(&self, d: &Delta, r: &mut Round) -> Result<()> {
        let (g, v) = (&self.graph, &self.v);
        // A new sameAs fact: what holds of one side now holds of the other.
        for &(x, y) in d.pairs(v.same) {
            self.check_budget()?;
            if x == y {
                continue;
            }
            let same = (x, v.same, y);
            r.add_with(g, (y, v.same, x), "eq-sym", || vec![same]);
            for &z in g.objects(y, v.same) {
                self.check_budget()?;
                if z != x && z != y {
                    r.add_with(g, (x, v.same, z), "eq-trans", || vec![same, (y, v.same, z)]);
                }
            }
            for &w in g.subjects(v.same, x) {
                self.check_budget()?;
                if w != x && w != y {
                    r.add_with(g, (w, v.same, y), "eq-trans", || vec![(w, v.same, x), same]);
                }
            }
            for &(p, o) in g.about(x) {
                self.check_budget()?;
                if p != v.same {
                    r.add_with(g, (y, p, o), "eq-rep-s", || vec![same, (x, p, o)]);
                }
            }
            for &(s, o) in g.pairs(x) {
                self.check_budget()?;
                r.add_with(g, (s, y, o), "eq-rep-p", || vec![same, (s, x, o)]);
            }
            for &(s, p) in g.mentioning(x) {
                self.check_budget()?;
                if p != v.same {
                    r.add_with(g, (s, p, y), "eq-rep-o", || vec![same, (s, p, x)]);
                }
            }
        }
        // A new fact, joined with the schema and equalities already known.
        for &t in &d.all {
            self.check_budget()?;
            let (s, p, o) = t;
            if p != v.same {
                for &s2 in g.objects(s, v.same) {
                    self.check_budget()?;
                    if s2 != s {
                        r.add_with(g, (s2, p, o), "eq-rep-s", || vec![(s, v.same, s2), t]);
                    }
                }
                for &o2 in g.objects(o, v.same) {
                    self.check_budget()?;
                    if o2 != o {
                        r.add_with(g, (s, p, o2), "eq-rep-o", || vec![(o, v.same, o2), t]);
                    }
                }
            }
            for &p2 in g.objects(p, v.same) {
                self.check_budget()?;
                if p2 != p {
                    r.add_with(g, (s, p2, o), "eq-rep-p", || vec![(p, v.same, p2), t]);
                }
            }
            for &c in g.objects(p, v.domain) {
                self.check_budget()?;
                r.add_with(g, (s, v.ty, c), "prp-dom", || vec![(p, v.domain, c), t]);
            }
            for &c in g.objects(p, v.range) {
                self.check_budget()?;
                r.add_with(g, (o, v.ty, c), "prp-rng", || vec![(p, v.range, c), t]);
            }
            for &p2 in g.objects(p, v.sub_prop) {
                self.check_budget()?;
                r.add_with(g, (s, p2, o), "prp-spo1", || vec![(p, v.sub_prop, p2), t]);
            }
            if p == v.ty {
                for &c2 in g.objects(o, v.sub_class) {
                    self.check_budget()?;
                    r.add_with(g, (s, v.ty, c2), "cax-sco", || vec![(o, v.sub_class, c2), t]);
                }
            }
            let declared = (p, v.ty, v.transitive);
            if g.has(declared) {
                for &z in g.objects(o, p) {
                    self.check_budget()?;
                    r.add_with(g, (s, p, z), "prp-trp", || vec![declared, t, (o, p, z)]);
                }
                for &w in g.subjects(p, s) {
                    self.check_budget()?;
                    r.add_with(g, (w, p, o), "prp-trp", || vec![declared, (w, p, s), t]);
                }
            }
            for (relation, rule) in [(v.sub_class, "scm-sco"), (v.sub_prop, "scm-spo")] {
                self.check_budget()?;
                if p == relation {
                    for &c in g.objects(o, relation) {
                        self.check_budget()?;
                        r.add_with(g, (s, relation, c), rule, || vec![t, (o, relation, c)]);
                    }
                    for &a in g.subjects(relation, s) {
                        self.check_budget()?;
                        r.add_with(g, (a, relation, o), rule, || vec![(a, relation, s), t]);
                    }
                }
            }
        }
        // A new schema fact, applied to everything already known.
        for &(p, c) in d.pairs(v.domain) {
            self.check_budget()?;
            for &(x, y) in g.pairs(p) {
                self.check_budget()?;
                r.add_with(g, (x, v.ty, c), "prp-dom", || vec![(p, v.domain, c), (x, p, y)]);
            }
        }
        for &(p, c) in d.pairs(v.range) {
            self.check_budget()?;
            for &(x, y) in g.pairs(p) {
                self.check_budget()?;
                r.add_with(g, (y, v.ty, c), "prp-rng", || vec![(p, v.range, c), (x, p, y)]);
            }
        }
        for &(p1, p2) in d.pairs(v.sub_prop) {
            self.check_budget()?;
            for &(x, y) in g.pairs(p1) {
                self.check_budget()?;
                r.add_with(g, (x, p2, y), "prp-spo1", || vec![(p1, v.sub_prop, p2), (x, p1, y)]);
            }
        }
        for &(c1, c2) in d.pairs(v.sub_class) {
            self.check_budget()?;
            for &x in self.instances(c1) {
                self.check_budget()?;
                r.add_with(g, (x, v.ty, c2), "cax-sco", || {
                    vec![(c1, v.sub_class, c2), (x, v.ty, c1)]
                });
            }
        }
        for &(p, kind) in d.pairs(v.ty) {
            self.check_budget()?;
            if kind == v.transitive {
                let declared = (p, v.ty, v.transitive);
                for &(x, y) in g.pairs(p) {
                    self.check_budget()?;
                    for &z in g.objects(y, p) {
                        self.check_budget()?;
                        r.add_with(g, (x, p, z), "prp-trp", || {
                            vec![declared, (x, p, y), (y, p, z)]
                        });
                    }
                }
            }
        }
        Ok(())
    }

    fn equality(&self, r: &mut Round) -> Result<()> {
        let (g, v) = (&self.graph, &self.v);
        for &(x, y) in g.pairs(v.same) {
            self.check_budget()?;
            let same = (x, v.same, y);
            if g.has((x, v.different, y)) {
                r.clash(
                    "eq-diff1",
                    format!(
                        "{} is both the same as and different from {}",
                        self.name(x),
                        self.name(y)
                    ),
                    vec![same, (x, v.different, y)],
                );
            }
        }
        for (rule, property) in [("eq-diff2", v.members), ("eq-diff3", v.distinct_members)] {
            self.check_budget()?;
            for &x in self.instances(v.all_different) {
                self.check_budget()?;
                for &list in g.objects(x, property) {
                    self.check_budget()?;
                    let Some((items, used)) = self.list(list) else { continue };
                    for (i, &a) in items.iter().enumerate() {
                        self.check_budget()?;
                        for &b in &items[i + 1..] {
                            self.check_budget()?;
                            if g.has((a, v.same, b)) {
                                let mut premises = vec![
                                    (x, v.ty, v.all_different),
                                    (x, property, list),
                                    (a, v.same, b),
                                ];
                                premises.extend(&used);
                                r.clash(
                                    rule,
                                    format!(
                                        "{} and {} are declared all different but are the same",
                                        self.name(a),
                                        self.name(b)
                                    ),
                                    premises,
                                );
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn properties(&self, r: &mut Round) -> Result<()> {
        let (g, v) = (&self.graph, &self.v);
        for &p in self.instances(v.functional) {
            self.check_budget()?;
            let declared = (p, v.ty, v.functional);
            for &(x, _) in g.pairs(p) {
                self.check_budget()?;
                let values = g.objects(x, p);
                for (i, &a) in values.iter().enumerate() {
                    self.check_budget()?;
                    for &b in &values[i + 1..] {
                        self.check_budget()?;
                        if self.distinct_values(a, b) {
                            r.clash(
                                "prp-fp",
                                format!(
                                    "{} is functional but {} has two values, {} and {}",
                                    self.name(p),
                                    self.name(x),
                                    self.name(a),
                                    self.name(b)
                                ),
                                vec![declared, (x, p, a), (x, p, b)],
                            );
                        } else if a != b && !self.terms.literal(a) && !self.terms.literal(b) {
                            r.add(
                                g,
                                (a, v.same, b),
                                "prp-fp",
                                vec![declared, (x, p, a), (x, p, b)],
                            );
                        }
                    }
                }
            }
        }
        for &p in self.instances(v.inverse_functional) {
            self.check_budget()?;
            let declared = (p, v.ty, v.inverse_functional);
            for &(_, y) in g.pairs(p) {
                self.check_budget()?;
                let holders = g.subjects(p, y);
                for (i, &a) in holders.iter().enumerate() {
                    self.check_budget()?;
                    for &b in &holders[i + 1..] {
                        self.check_budget()?;
                        if a != b {
                            r.add(
                                g,
                                (a, v.same, b),
                                "prp-ifp",
                                vec![declared, (a, p, y), (b, p, y)],
                            );
                        }
                    }
                }
            }
        }
        for &p in self.instances(v.irreflexive) {
            self.check_budget()?;
            for &(x, y) in g.pairs(p) {
                self.check_budget()?;
                if x == y {
                    r.clash(
                        "prp-irp",
                        format!(
                            "{} is irreflexive but {} relates to itself",
                            self.name(p),
                            self.name(x)
                        ),
                        vec![(p, v.ty, v.irreflexive), (x, p, x)],
                    );
                }
            }
        }
        for &p in self.instances(v.symmetric) {
            self.check_budget()?;
            for &(x, y) in g.pairs(p) {
                self.check_budget()?;
                if !self.terms.literal(y) {
                    r.add(g, (y, p, x), "prp-symp", vec![(p, v.ty, v.symmetric), (x, p, y)]);
                }
            }
        }
        for &p in self.instances(v.asymmetric) {
            self.check_budget()?;
            for &(x, y) in g.pairs(p) {
                self.check_budget()?;
                if g.has((y, p, x)) {
                    r.clash(
                        "prp-asyp",
                        format!(
                            "{} is asymmetric but holds both ways between {} and {}",
                            self.name(p),
                            self.name(x),
                            self.name(y)
                        ),
                        vec![(p, v.ty, v.asymmetric), (x, p, y), (y, p, x)],
                    );
                }
            }
        }
        for &(p, list) in g.pairs(v.chain) {
            self.check_budget()?;
            let Some((links, used)) = self.list(list) else { continue };
            let Some(&first) = links.first() else { continue };
            // Paths u0 -p1-> u1 ... grown one link at a time, keeping the triples walked.
            let mut paths: Vec<(Id, Id, Vec<T>)> =
                g.pairs(first).iter().map(|&(a, b)| (a, b, vec![(a, first, b)])).collect();
            ensure!(
                paths.len() <= MAX_CHAIN_PATHS,
                "property chain exceeds {MAX_CHAIN_PATHS} intermediate paths"
            );
            for &link in &links[1..] {
                self.check_budget()?;
                let mut next = Vec::new();
                for (start, end, walked) in &paths {
                    self.check_budget()?;
                    for &o in g.objects(*end, link) {
                        self.check_budget()?;
                        // A prefix of a chain never justifies its conclusion. Abort the
                        // entire computation before allocating beyond the path budget.
                        ensure!(
                            next.len() < MAX_CHAIN_PATHS,
                            "property chain exceeds {MAX_CHAIN_PATHS} intermediate paths"
                        );
                        let mut walked = walked.clone();
                        walked.push((*end, link, o));
                        next.push((*start, o, walked));
                    }
                }
                paths = next;
            }
            for (start, end, walked) in paths {
                self.check_budget()?;
                let mut premises = vec![(p, v.chain, list)];
                premises.extend(&used);
                premises.extend(walked);
                r.add(g, (start, p, end), "prp-spo2", premises);
            }
        }
        for &(p1, p2) in g.pairs(v.eq_prop) {
            self.check_budget()?;
            for &(x, y) in g.pairs(p1) {
                self.check_budget()?;
                r.add(g, (x, p2, y), "prp-eqp1", vec![(p1, v.eq_prop, p2), (x, p1, y)]);
            }
            for &(x, y) in g.pairs(p2) {
                self.check_budget()?;
                r.add(g, (x, p1, y), "prp-eqp2", vec![(p1, v.eq_prop, p2), (x, p2, y)]);
            }
        }
        for &(p1, p2) in g.pairs(v.prop_disjoint) {
            self.check_budget()?;
            for &(x, y) in g.pairs(p1) {
                self.check_budget()?;
                if g.has((x, p2, y)) {
                    r.clash(
                        "prp-pdw",
                        format!(
                            "{} and {} are disjoint properties but both relate {} to {}",
                            self.name(p1),
                            self.name(p2),
                            self.name(x),
                            self.name(y)
                        ),
                        vec![(p1, v.prop_disjoint, p2), (x, p1, y), (x, p2, y)],
                    );
                }
            }
        }
        for &x in self.instances(v.all_disjoint_props) {
            self.check_budget()?;
            for &list in g.objects(x, v.members) {
                self.check_budget()?;
                let Some((props, used)) = self.list(list) else { continue };
                for (i, &a) in props.iter().enumerate() {
                    self.check_budget()?;
                    for &b in &props[i + 1..] {
                        self.check_budget()?;
                        for &(u, w) in g.pairs(a) {
                            self.check_budget()?;
                            if a != b && g.has((u, b, w)) {
                                let mut premises = vec![
                                    (x, v.ty, v.all_disjoint_props),
                                    (x, v.members, list),
                                    (u, a, w),
                                    (u, b, w),
                                ];
                                premises.extend(&used);
                                r.clash("prp-adp", format!("{} and {} are disjoint properties but both relate {} to {}", self.name(a), self.name(b), self.name(u), self.name(w)), premises);
                            }
                        }
                    }
                }
            }
        }
        for &(p1, p2) in g.pairs(v.inverse) {
            self.check_budget()?;
            for &(x, y) in g.pairs(p1) {
                self.check_budget()?;
                if !self.terms.literal(y) {
                    r.add(g, (y, p2, x), "prp-inv1", vec![(p1, v.inverse, p2), (x, p1, y)]);
                }
            }
            for &(x, y) in g.pairs(p2) {
                self.check_budget()?;
                if !self.terms.literal(y) {
                    r.add(g, (y, p1, x), "prp-inv2", vec![(p1, v.inverse, p2), (x, p2, y)]);
                }
            }
        }
        for &(c, list) in g.pairs(v.key) {
            self.check_budget()?;
            let Some((keys, used)) = self.list(list) else { continue };
            if keys.is_empty() {
                continue;
            }
            let members = self.with_type(c);
            // Group the members of c by their values for every key property.
            let mut groups: HashMap<Vec<Id>, Vec<(Id, Vec<T>)>> = HashMap::new();
            for &x in &members {
                self.check_budget()?;
                let mut choices: Vec<(Vec<Id>, Vec<T>)> = vec![(Vec::new(), Vec::new())];
                for &k in &keys {
                    self.check_budget()?;
                    let mut next = Vec::new();
                    for (values, support) in &choices {
                        self.check_budget()?;
                        for &z in g.objects(x, k) {
                            self.check_budget()?;
                            let (mut values, mut support) = (values.clone(), support.clone());
                            values.push(z);
                            support.push((x, k, z));
                            ensure!(
                                next.len() < 100_000,
                                "key expansion exceeds intermediate combination limit"
                            );
                            next.push((values, support));
                        }
                    }
                    choices = next;
                }
                for (values, support) in choices {
                    self.check_budget()?;
                    groups.entry(values).or_default().push((x, support));
                }
            }
            for holders in groups.values() {
                self.check_budget()?;
                for (i, (a, sa)) in holders.iter().enumerate() {
                    self.check_budget()?;
                    for (b, sb) in &holders[i + 1..] {
                        self.check_budget()?;
                        if a != b {
                            let mut premises = vec![(c, v.key, list), (*a, v.ty, c), (*b, v.ty, c)];
                            premises.extend(&used);
                            premises.extend(sa);
                            premises.extend(sb);
                            r.add(g, (*a, v.same, *b), "prp-key", premises);
                        }
                    }
                }
            }
        }
        for &(x, i1) in g.pairs(v.npa_source) {
            self.check_budget()?;
            for &p in g.objects(x, v.npa_property) {
                self.check_budget()?;
                for (property, rule) in [(v.npa_target, "prp-npa1"), (v.npa_value, "prp-npa2")] {
                    self.check_budget()?;
                    for &i2 in g.objects(x, property) {
                        self.check_budget()?;
                        if g.has((i1, p, i2)) {
                            r.clash(
                                rule,
                                format!(
                                    "{} {} {} is asserted and also negated",
                                    self.name(i1),
                                    self.name(p),
                                    self.name(i2)
                                ),
                                vec![
                                    (x, v.npa_source, i1),
                                    (x, v.npa_property, p),
                                    (x, property, i2),
                                    (i1, p, i2),
                                ],
                            );
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn classes(&self, r: &mut Round) -> Result<()> {
        let (g, v) = (&self.graph, &self.v);
        for &x in self.instances(v.nothing) {
            self.check_budget()?;
            r.clash(
                "cls-nothing2",
                format!("{} is an instance of owl:Nothing", self.name(x)),
                vec![(x, v.ty, v.nothing)],
            );
        }
        for &(c, list) in g.pairs(v.intersection) {
            self.check_budget()?;
            let Some((parts, used)) = self.list(list) else { continue };
            let Some(&first) = parts.first() else { continue };
            for &y in self.instances(first) {
                self.check_budget()?;
                if parts.iter().all(|&ci| self.typed(y, ci)) {
                    let mut premises = vec![(c, v.intersection, list)];
                    premises.extend(&used);
                    premises.extend(parts.iter().map(|&ci| (y, v.ty, ci)));
                    r.add(g, (y, v.ty, c), "cls-int1", premises);
                }
            }
            for &y in self.instances(c) {
                self.check_budget()?;
                for &ci in &parts {
                    self.check_budget()?;
                    let mut premises = vec![(c, v.intersection, list), (y, v.ty, c)];
                    premises.extend(&used);
                    r.add(g, (y, v.ty, ci), "cls-int2", premises);
                }
            }
        }
        for &(c, list) in g.pairs(v.union) {
            self.check_budget()?;
            let Some((parts, used)) = self.list(list) else { continue };
            for &ci in &parts {
                self.check_budget()?;
                for &y in self.instances(ci) {
                    self.check_budget()?;
                    let mut premises = vec![(c, v.union, list), (y, v.ty, ci)];
                    premises.extend(&used);
                    r.add(g, (y, v.ty, c), "cls-uni", premises);
                }
            }
        }
        for &(c1, c2) in g.pairs(v.complement) {
            self.check_budget()?;
            for &x in self.instances(c1) {
                self.check_budget()?;
                if self.typed(x, c2) {
                    r.clash(
                        "cls-com",
                        format!(
                            "{} is in both {} and its complement {}",
                            self.name(x),
                            self.name(c2),
                            self.name(c1)
                        ),
                        vec![(c1, v.complement, c2), (x, v.ty, c1), (x, v.ty, c2)],
                    );
                }
            }
        }
        for &(x, y) in g.pairs(v.some) {
            self.check_budget()?;
            for &p in g.objects(x, v.on_property) {
                self.check_budget()?;
                for &(u, w) in g.pairs(p) {
                    self.check_budget()?;
                    if y == v.thing {
                        r.add(
                            g,
                            (u, v.ty, x),
                            "cls-svf2",
                            vec![(x, v.some, y), (x, v.on_property, p), (u, p, w)],
                        );
                    } else if self.typed(w, y) {
                        r.add(
                            g,
                            (u, v.ty, x),
                            "cls-svf1",
                            vec![(x, v.some, y), (x, v.on_property, p), (u, p, w), (w, v.ty, y)],
                        );
                    }
                }
            }
        }
        for &(x, y) in g.pairs(v.all) {
            self.check_budget()?;
            for &p in g.objects(x, v.on_property) {
                self.check_budget()?;
                for &u in self.instances(x) {
                    self.check_budget()?;
                    for &w in g.objects(u, p) {
                        self.check_budget()?;
                        if !self.terms.literal(w) {
                            r.add(
                                g,
                                (w, v.ty, y),
                                "cls-avf",
                                vec![(x, v.all, y), (x, v.on_property, p), (u, v.ty, x), (u, p, w)],
                            );
                        }
                    }
                }
            }
        }
        for &(x, y) in g.pairs(v.has_value) {
            self.check_budget()?;
            for &p in g.objects(x, v.on_property) {
                self.check_budget()?;
                for &u in self.instances(x) {
                    self.check_budget()?;
                    r.add(
                        g,
                        (u, p, y),
                        "cls-hv1",
                        vec![(x, v.has_value, y), (x, v.on_property, p), (u, v.ty, x)],
                    );
                }
                for &u in g.subjects(p, y) {
                    self.check_budget()?;
                    r.add(
                        g,
                        (u, v.ty, x),
                        "cls-hv2",
                        vec![(x, v.has_value, y), (x, v.on_property, p), (u, p, y)],
                    );
                }
            }
        }
        for &(x, n) in g.pairs(v.max_card) {
            self.check_budget()?;
            let Some(limit @ 0..=1) = self.terms.number(n) else { continue };
            for &p in g.objects(x, v.on_property) {
                self.check_budget()?;
                for &u in self.instances(x) {
                    self.check_budget()?;
                    let values = g.objects(u, p);
                    if limit == 0 {
                        if let Some(&w) = values.first() {
                            r.clash(
                                "cls-maxc1",
                                format!(
                                    "{} may have no {} but has {}",
                                    self.name(u),
                                    self.name(p),
                                    self.name(w)
                                ),
                                vec![
                                    (x, v.max_card, n),
                                    (x, v.on_property, p),
                                    (u, v.ty, x),
                                    (u, p, w),
                                ],
                            );
                        }
                        continue;
                    }
                    for (i, &a) in values.iter().enumerate() {
                        self.check_budget()?;
                        for &b in &values[i + 1..] {
                            self.check_budget()?;
                            if self.distinct_values(a, b) {
                                r.clash(
                                    "cls-maxc2",
                                    format!(
                                        "{} may have one {} but has {} and {}",
                                        self.name(u),
                                        self.name(p),
                                        self.name(a),
                                        self.name(b)
                                    ),
                                    vec![
                                        (x, v.max_card, n),
                                        (x, v.on_property, p),
                                        (u, v.ty, x),
                                        (u, p, a),
                                        (u, p, b),
                                    ],
                                );
                            } else if a != b && !self.terms.literal(a) && !self.terms.literal(b) {
                                r.add(
                                    g,
                                    (a, v.same, b),
                                    "cls-maxc2",
                                    vec![
                                        (x, v.max_card, n),
                                        (x, v.on_property, p),
                                        (u, v.ty, x),
                                        (u, p, a),
                                        (u, p, b),
                                    ],
                                );
                            }
                        }
                    }
                }
            }
        }
        for &(x, n) in g.pairs(v.max_qcard) {
            self.check_budget()?;
            let Some(limit @ 0..=1) = self.terms.number(n) else { continue };
            for &p in g.objects(x, v.on_property) {
                self.check_budget()?;
                for &c in g.objects(x, v.on_class) {
                    self.check_budget()?;
                    let any = c == v.thing;
                    for &u in self.instances(x) {
                        self.check_budget()?;
                        let qualified: Vec<Id> = g
                            .objects(u, p)
                            .iter()
                            .copied()
                            .filter(|&w| any || self.typed(w, c))
                            .collect();
                        let support = |w: Id| {
                            if any { vec![(u, p, w)] } else { vec![(u, p, w), (w, v.ty, c)] }
                        };
                        let header = vec![
                            (x, v.max_qcard, n),
                            (x, v.on_property, p),
                            (x, v.on_class, c),
                            (u, v.ty, x),
                        ];
                        if limit == 0 {
                            if let Some(&w) = qualified.first() {
                                let mut premises = header.clone();
                                premises.extend(support(w));
                                r.clash(
                                    if any { "cls-maxqc2" } else { "cls-maxqc1" },
                                    format!(
                                        "{} may have no {} in {} but has {}",
                                        self.name(u),
                                        self.name(p),
                                        self.name(c),
                                        self.name(w)
                                    ),
                                    premises,
                                );
                            }
                            continue;
                        }
                        for (i, &a) in qualified.iter().enumerate() {
                            self.check_budget()?;
                            for &b in &qualified[i + 1..] {
                                self.check_budget()?;
                                if self.distinct_values(a, b) {
                                    let mut premises = header.clone();
                                    premises.extend(support(a));
                                    premises.extend(support(b));
                                    r.clash(
                                        if any { "cls-maxqc4" } else { "cls-maxqc3" },
                                        format!(
                                            "{} may have one {} but has {} and {}",
                                            self.name(u),
                                            self.name(p),
                                            self.name(a),
                                            self.name(b)
                                        ),
                                        premises,
                                    );
                                } else if a != b && !self.terms.literal(a) && !self.terms.literal(b)
                                {
                                    let mut premises = header.clone();
                                    premises.extend(support(a));
                                    premises.extend(support(b));
                                    r.add(
                                        g,
                                        (a, v.same, b),
                                        if any { "cls-maxqc4" } else { "cls-maxqc3" },
                                        premises,
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
        for &(c, list) in g.pairs(v.one_of) {
            self.check_budget()?;
            let Some((items, used)) = self.list(list) else { continue };
            for &y in &items {
                self.check_budget()?;
                if !self.terms.literal(y) {
                    let mut premises = vec![(c, v.one_of, list)];
                    premises.extend(&used);
                    r.add(g, (y, v.ty, c), "cls-oo", premises);
                }
            }
        }
        Ok(())
    }

    fn class_axioms(&self, r: &mut Round) -> Result<()> {
        let (g, v) = (&self.graph, &self.v);
        for &(c1, c2) in g.pairs(v.eq_class) {
            self.check_budget()?;
            for &x in self.instances(c1) {
                self.check_budget()?;
                r.add(g, (x, v.ty, c2), "cax-eqc1", vec![(c1, v.eq_class, c2), (x, v.ty, c1)]);
            }
            for &x in self.instances(c2) {
                self.check_budget()?;
                r.add(g, (x, v.ty, c1), "cax-eqc2", vec![(c1, v.eq_class, c2), (x, v.ty, c2)]);
            }
        }
        for &(c1, c2) in g.pairs(v.disjoint) {
            self.check_budget()?;
            for &x in self.instances(c1) {
                self.check_budget()?;
                if self.typed(x, c2) {
                    r.clash(
                        "cax-dw",
                        format!(
                            "{} is in both {} and {}, which are disjoint",
                            self.name(x),
                            self.name(c1),
                            self.name(c2)
                        ),
                        vec![(c1, v.disjoint, c2), (x, v.ty, c1), (x, v.ty, c2)],
                    );
                }
            }
        }
        for &x in self.instances(v.all_disjoint_classes) {
            self.check_budget()?;
            for &list in g.objects(x, v.members) {
                self.check_budget()?;
                let Some((classes, used)) = self.list(list) else { continue };
                for (i, &a) in classes.iter().enumerate() {
                    self.check_budget()?;
                    for &b in &classes[i + 1..] {
                        self.check_budget()?;
                        for &z in self.instances(a) {
                            self.check_budget()?;
                            if a != b && self.typed(z, b) {
                                let mut premises = vec![
                                    (x, v.ty, v.all_disjoint_classes),
                                    (x, v.members, list),
                                    (z, v.ty, a),
                                    (z, v.ty, b),
                                ];
                                premises.extend(&used);
                                r.clash(
                                    "cax-adc",
                                    format!(
                                        "{} is in both {} and {}, which are disjoint",
                                        self.name(z),
                                        self.name(a),
                                        self.name(b)
                                    ),
                                    premises,
                                );
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn schema(&self, r: &mut Round) -> Result<()> {
        let (g, v) = (&self.graph, &self.v);
        for &c in self.instances(v.class) {
            self.check_budget()?;
            let declared = (c, v.ty, v.class);
            r.add(g, (c, v.sub_class, c), "scm-cls", vec![declared]);
            r.add(g, (c, v.eq_class, c), "scm-cls", vec![declared]);
            r.add(g, (c, v.sub_class, v.thing), "scm-cls", vec![declared]);
            r.add(g, (v.nothing, v.sub_class, c), "scm-cls", vec![declared]);
        }
        for (kind, rule) in [(v.object_property, "scm-op"), (v.datatype_property, "scm-dp")] {
            self.check_budget()?;
            for &p in self.instances(kind) {
                self.check_budget()?;
                r.add(g, (p, v.sub_prop, p), rule, vec![(p, v.ty, kind)]);
                r.add(g, (p, v.eq_prop, p), rule, vec![(p, v.ty, kind)]);
            }
        }
        for (equivalent, sub, rule1, rule2) in [
            (v.eq_class, v.sub_class, "scm-eqc1", "scm-eqc2"),
            (v.eq_prop, v.sub_prop, "scm-eqp1", "scm-eqp2"),
        ] {
            for &(a, b) in g.pairs(equivalent) {
                self.check_budget()?;
                r.add(g, (a, sub, b), rule1, vec![(a, equivalent, b)]);
                r.add(g, (b, sub, a), rule1, vec![(a, equivalent, b)]);
            }
            for &(a, b) in g.pairs(sub) {
                self.check_budget()?;
                if g.has((b, sub, a)) {
                    r.add(g, (a, equivalent, b), rule2, vec![(a, sub, b), (b, sub, a)]);
                }
            }
        }
        for (relation, rule1, rule2) in
            [(v.domain, "scm-dom1", "scm-dom2"), (v.range, "scm-rng1", "scm-rng2")]
        {
            for &(p, c1) in g.pairs(relation) {
                self.check_budget()?;
                for &c2 in g.objects(c1, v.sub_class) {
                    self.check_budget()?;
                    r.add(
                        g,
                        (p, relation, c2),
                        rule1,
                        vec![(p, relation, c1), (c1, v.sub_class, c2)],
                    );
                }
                for &p1 in g.subjects(v.sub_prop, p) {
                    self.check_budget()?;
                    r.add(
                        g,
                        (p1, relation, c1),
                        rule2,
                        vec![(p, relation, c1), (p1, v.sub_prop, p)],
                    );
                }
            }
        }
        // Restrictions compared pairwise: scm-hv, scm-svf1/2, scm-avf1/2.
        for (filler, rules) in [
            (v.has_value, ["scm-hv", "scm-hv"]),
            (v.some, ["scm-svf1", "scm-svf2"]),
            (v.all, ["scm-avf1", "scm-avf2"]),
        ] {
            let restrictions: Vec<(Id, Id, Id)> = g
                .pairs(filler)
                .iter()
                .flat_map(|&(c, y)| g.objects(c, v.on_property).iter().map(move |&p| (c, y, p)))
                .collect();
            for &(c1, y1, p1) in &restrictions {
                self.check_budget()?;
                for &(c2, y2, p2) in &restrictions {
                    self.check_budget()?;
                    if c1 == c2 {
                        continue;
                    }
                    let header = vec![
                        (c1, filler, y1),
                        (c1, v.on_property, p1),
                        (c2, filler, y2),
                        (c2, v.on_property, p2),
                    ];
                    // Same property, fillers in a subclass relation (not for hasValue).
                    if p1 == p2 && filler != v.has_value && y1 != y2 && g.has((y1, v.sub_class, y2))
                    {
                        let mut premises = header.clone();
                        premises.push((y1, v.sub_class, y2));
                        r.add(g, (c1, v.sub_class, c2), rules[0], premises);
                    }
                    // Same filler, properties in a subproperty relation.
                    if y1 == y2 && p1 != p2 && g.has((p1, v.sub_prop, p2)) {
                        let mut premises = header.clone();
                        premises.push((p1, v.sub_prop, p2));
                        let (sub, sup) = if filler == v.all { (c2, c1) } else { (c1, c2) };
                        r.add(g, (sub, v.sub_class, sup), rules[1], premises);
                    }
                }
            }
        }
        for (relation, rule) in [(v.intersection, "scm-int"), (v.union, "scm-uni")] {
            self.check_budget()?;
            for &(c, list) in g.pairs(relation) {
                self.check_budget()?;
                let Some((parts, used)) = self.list(list) else { continue };
                for &ci in &parts {
                    self.check_budget()?;
                    let mut premises = vec![(c, relation, list)];
                    premises.extend(&used);
                    let t = if relation == v.intersection {
                        (c, v.sub_class, ci)
                    } else {
                        (ci, v.sub_class, c)
                    };
                    r.add(g, t, rule, premises);
                }
            }
        }
        Ok(())
    }

    /// Two literals that cannot be the same data value: same datatype (and language), different
    /// value. Literals of different datatypes are left alone (`"1"^^xsd:int` = `"1"^^xsd:integer`).
    fn distinct_values(&self, a: Id, b: Id) -> bool {
        let (Term::Literal(x), Term::Literal(y)) = (self.terms.term(a), self.terms.term(b)) else {
            return false;
        };
        x.datatype() == y.datatype()
            && x.language() == y.language()
            // Unknown value spaces must not be declared unequal by lexical form.
            && (x.language().is_some() || x.datatype().as_str() == format!("{XSD}string")
                || x.datatype().as_str().strip_prefix(XSD).is_some_and(|kind| match kind {
                    "integer" | "int" | "long" | "short" | "byte" | "nonNegativeInteger" | "positiveInteger" | "nonPositiveInteger" | "negativeInteger" | "unsignedLong" | "unsignedInt" | "unsignedShort" | "unsignedByte" => x.value().trim().parse::<i128>().is_ok() && y.value().trim().parse::<i128>().is_ok(),
                    "decimal" | "float" | "double" => x.value().trim().parse::<f64>().is_ok() && y.value().trim().parse::<f64>().is_ok(),
                    "boolean" => ["true", "false", "0", "1"].contains(&x.value().trim()) && ["true", "false", "0", "1"].contains(&y.value().trim()),
                    _ => false,
                }))
            && by_value(x.clone().into()) != by_value(y.clone().into())
    }
    fn name(&self, id: Id) -> String {
        match self.terms.term(id) {
            Term::NamedNode(n) => n.as_str().to_owned(),
            other => other.to_string(),
        }
    }
    fn vocabulary(&self, id: Id) -> bool {
        matches!(self.terms.term(id), Term::NamedNode(n) if [RDF, RDFS, OWL, XSD].iter().any(|ns| n.as_str().starts_with(ns)))
    }
    fn mentioned(&self) -> HashSet<Id> {
        self.input.iter().flat_map(|&(s, p, o)| [s, p, o]).collect()
    }
    /// The `owlrl-full-v1` output policy.
    fn shown(&self, t: T, mentioned: &HashSet<Id>) -> bool {
        let (s, p, o) = t;
        let everything = [self.v.ty, self.v.sub_class, self.v.domain, self.v.range].contains(&p)
            && (o == self.v.thing || self.name(o) == format!("{RDFS}Resource"));
        !everything && (!self.vocabulary(s) || mentioned.contains(&s))
    }

    pub fn clashes(&self) -> Option<Inconsistent> {
        if self.clashes.is_empty() {
            return None;
        }
        let clashes = self
            .clashes
            .iter()
            .take(MAX_CLASHES)
            .map(|c| {
                let mut terms: Vec<String> = Vec::new();
                for &(s, p, o) in &c.premises {
                    for id in [s, o] {
                        if let Term::NamedNode(n) = self.terms.term(id) {
                            let name = n.as_str().to_owned();
                            if !self.vocabulary(id) && p != self.v.first && p != self.v.rest && !terms.contains(&name) {
                                terms.push(name);
                            }
                        }
                    }
                }
                json!({"rule": c.rule, "message": c.message, "terms": terms,
                       "premises": c.premises.iter().map(|&t| self.terms.line(t)).collect::<Vec<_>>()})
            })
            .collect();
        Some(Inconsistent { clashes })
    }
    /// Derived triples under the output policy, sorted, as N-Triples.
    pub fn derived(&self) -> (String, usize) {
        let mentioned = self.mentioned();
        let mut lines: Vec<String> = self
            .why
            .keys()
            .filter(|&&t| !self.input.contains(&t) && self.shown(t, &mentioned))
            .map(|&t| self.terms.line(t))
            .collect();
        lines.sort();
        let count = lines.len();
        (lines.join("\n"), count)
    }
    /// How `target` was derived: the rule and its premises, each premise either an input line or,
    /// when this closure derived it too, its own derivation, to a bounded depth and size.
    pub fn explain(&mut self, target: &str) -> Result<Vec<Value>> {
        let mut quads = RdfParser::from_format(RdfFormat::NTriples).for_reader(target.as_bytes());
        let quad = quads.next().context("no triple to explain")??;
        ensure!(quads.next().is_none(), "explain one triple at a time");
        let triple = Triple::new(quad.subject, quad.predicate, quad.object);
        let ids =
            (&triple.subject.clone().into(), &triple.predicate.clone().into(), &triple.object);
        let (Some(&s), Some(&p), Some(&o)) =
            (self.terms.ids.get(ids.0), self.terms.ids.get(ids.1), self.terms.ids.get(ids.2))
        else {
            return Ok(Vec::new());
        };
        let t = (s, p, o);
        if self.input.contains(&t) || !self.why.contains_key(&t) {
            return Ok(Vec::new());
        }
        let mut budget = MAX_PROOF_NODES;
        let mut path = HashSet::new();
        let proof = self.proof(t, 0, &mut budget, &mut path);
        Ok(vec![json!({
            "rule": proof["rule"], "rule_text": rule_text(proof["rule"].as_str().unwrap_or_default()),
            "bindings": {}, "premises": proof["premises"], "aggregate": false, "incomplete": proof["truncated"] == true,
        })])
    }
    fn proof(&self, t: T, depth: usize, budget: &mut usize, path: &mut HashSet<T>) -> Value {
        let Some(why) = self.why.get(&t) else { return json!({"triple": self.terms.line(t)}) };
        let mut truncated = false;
        let premises: Vec<Value> = why
            .premises
            .iter()
            .map(|&premise| {
                if self.input.contains(&premise) || !self.why.contains_key(&premise) {
                    return json!(self.terms.line(premise));
                }
                if depth + 1 >= MAX_PROOF_DEPTH || *budget == 0 || !path.insert(premise) {
                    truncated = true;
                    return json!({"triple": self.terms.line(premise), "rule": self.why[&premise].rule, "truncated": true});
                }
                *budget -= 1;
                let inner = self.proof(premise, depth + 1, budget, path);
                path.remove(&premise);
                truncated |= inner["truncated"] == true;
                json!({"triple": self.terms.line(premise), "rule": inner["rule"], "premises": inner["premises"]})
            })
            .collect();
        json!({"rule": why.rule, "premises": premises, "truncated": truncated})
    }
}

/// The rule as the OWL 2 RL/RDF tables state it, for readers of an explanation.
fn rule_text(rule: &str) -> &'static str {
    match rule {
        "eq-ref" => "every resource is the same as itself",
        "eq-sym" => "x sameAs y => y sameAs x",
        "eq-trans" => "x sameAs y, y sameAs z => x sameAs z",
        "eq-rep-s" | "eq-rep-p" | "eq-rep-o" => "x sameAs y => what holds of x holds of y",
        "prp-dom" => "p domain c, x p y => x a c",
        "prp-rng" => "p range c, x p y => y a c",
        "prp-fp" => "p a FunctionalProperty, x p y1, x p y2 => y1 sameAs y2",
        "prp-ifp" => "p a InverseFunctionalProperty, x1 p y, x2 p y => x1 sameAs x2",
        "prp-symp" => "p a SymmetricProperty, x p y => y p x",
        "prp-trp" => "p a TransitiveProperty, x p y, y p z => x p z",
        "prp-spo1" => "p1 subPropertyOf p2, x p1 y => x p2 y",
        "prp-spo2" => "p propertyChainAxiom (p1 ... pn), x p1 ... pn y => x p y",
        "prp-eqp1" | "prp-eqp2" => "p1 equivalentProperty p2: each holds wherever the other does",
        "prp-inv1" | "prp-inv2" => "p1 inverseOf p2, x p1 y => y p2 x",
        "prp-key" => "c hasKey (p1 ... pn), x and y in c with equal key values => x sameAs y",
        "cls-int1" => "c intersectionOf (c1 ... cn), y in every ci => y a c",
        "cls-int2" => "c intersectionOf (c1 ... cn), y a c => y a ci",
        "cls-uni" => "c unionOf (c1 ... cn), y a ci => y a c",
        "cls-svf1" => "x someValuesFrom y onProperty p, u p v, v a y => u a x",
        "cls-svf2" => "x someValuesFrom owl:Thing onProperty p, u p v => u a x",
        "cls-avf" => "x allValuesFrom y onProperty p, u a x, u p v => v a y",
        "cls-hv1" => "x hasValue y onProperty p, u a x => u p y",
        "cls-hv2" => "x hasValue y onProperty p, u p y => u a x",
        "cls-maxc2" | "cls-maxqc3" | "cls-maxqc4" => {
            "at most one value, two values => they are the same"
        }
        "cls-oo" => "c oneOf (y1 ... yn) => yi a c",
        "cax-sco" => "c1 subClassOf c2, x a c1 => x a c2",
        "cax-eqc1" | "cax-eqc2" => {
            "c1 equivalentClass c2: members of each are members of the other"
        }
        "scm-cls" => {
            "every class is a subclass and equivalent of itself, between owl:Nothing and owl:Thing"
        }
        "scm-op" | "scm-dp" => "every property is a subproperty and equivalent of itself",
        "scm-sco" => "subClassOf is transitive",
        "scm-spo" => "subPropertyOf is transitive",
        "scm-eqc1" | "scm-eqp1" => "equivalence means subsumption both ways",
        "scm-eqc2" | "scm-eqp2" => "subsumption both ways means equivalence",
        "scm-dom1" | "scm-rng1" => "a domain/range extends to superclasses",
        "scm-dom2" | "scm-rng2" => "a domain/range carries to subproperties",
        "scm-hv" | "scm-svf1" | "scm-svf2" | "scm-avf1" | "scm-avf2" => {
            "restrictions on related properties or fillers are subclasses"
        }
        "scm-int" => "an intersection is a subclass of each part",
        "scm-uni" => "each part is a subclass of the union",
        _ => "",
    }
}

/// Limits a closure runs under.
pub struct Limits {
    pub max_derived: usize,
    pub timeout: Duration,
}
pub fn infer(ntriples: &str, limits: &Limits) -> Result<(String, usize)> {
    let closure = Closure::compute(ntriples, limits.max_derived, limits.timeout)?;
    if let Some(inconsistent) = closure.clashes() {
        return Err(inconsistent.into());
    }
    Ok(closure.derived())
}
pub fn explain(ntriples: &str, limits: &Limits, target: &str) -> Result<Vec<Value>> {
    let mut closure = Closure::compute(ntriples, limits.max_derived, limits.timeout)?;
    closure.explain(target)
}
