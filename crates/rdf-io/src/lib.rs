//! RDF parser and serializer entry points.

#![forbid(unsafe_code)]

use logos::Logos;
use rdf_model::{
    BlankNode, GraphName, Literal, ModelError, NamedNode, Object, Predicate, Quad, Subject, Triple,
    XSD_STRING_IRI,
};
use std::char;
use std::collections::HashMap;
use std::io::{self, BufRead, Cursor, Write};
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum NTriplesErrorKind {
    #[error("I/O error")]
    Io,
    #[error("expected subject")]
    ExpectedSubject,
    #[error("expected predicate")]
    ExpectedPredicate,
    #[error("expected object")]
    ExpectedObject,
    #[error("expected whitespace")]
    ExpectedWhitespace,
    #[error("expected terminating dot")]
    ExpectedDot,
    #[error("invalid IRI")]
    InvalidIri,
    #[error("invalid blank node")]
    InvalidBlankNode,
    #[error("invalid literal")]
    InvalidLiteral,
    #[error("invalid language tag")]
    InvalidLanguageTag,
    #[error("invalid escape sequence")]
    InvalidEscape,
    #[error("unexpected trailing characters")]
    TrailingCharacters,
    #[error("unexpected end of input")]
    UnexpectedEof,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("{kind} at line {line}, column {column}: {message}")]
pub struct NTriplesError {
    pub kind: NTriplesErrorKind,
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl NTriplesError {
    fn new(
        kind: NTriplesErrorKind,
        line: usize,
        column: usize,
        message: impl Into<String>,
    ) -> Self {
        Self { kind, line, column, message: message.into() }
    }
}

pub struct NTriplesParser<R: BufRead> {
    lines: io::Lines<R>,
    line_number: usize,
}

impl<R: BufRead> Iterator for NTriplesParser<R> {
    type Item = Result<Triple, NTriplesError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let raw_line = self.lines.next()?;
            self.line_number += 1;
            match raw_line {
                Ok(line) => match parse_ntriples_line(&line, self.line_number) {
                    Ok(Some(triple)) => return Some(Ok(triple)),
                    Ok(None) => {}
                    Err(error) => return Some(Err(error)),
                },
                Err(error) => {
                    return Some(Err(NTriplesError::new(
                        NTriplesErrorKind::Io,
                        self.line_number,
                        1,
                        error.to_string(),
                    )));
                }
            }
        }
    }
}

pub fn parse_ntriples_reader<R: BufRead>(reader: R) -> NTriplesParser<R> {
    NTriplesParser { lines: reader.lines(), line_number: 0 }
}

pub fn parse_ntriples(input: &str) -> Result<Vec<Triple>, NTriplesError> {
    parse_ntriples_reader(Cursor::new(input.as_bytes())).collect()
}

pub fn serialize_ntriples<W: Write>(writer: &mut W, triples: &[Triple]) -> io::Result<()> {
    for triple in triples {
        writeln!(
            writer,
            "{} {} {} .",
            serialize_subject(triple.subject()),
            serialize_named_node(triple.predicate().as_named_node()),
            serialize_object(triple.object())
        )?;
    }
    Ok(())
}

pub fn serialize_ntriples_to_string(triples: &[Triple]) -> io::Result<String> {
    let mut buffer = Vec::new();
    serialize_ntriples(&mut buffer, triples)?;
    // Serializer only writes UTF-8 compatible content.
    Ok(String::from_utf8(buffer).expect("serializer emitted invalid UTF-8"))
}

fn serialize_subject(subject: &Subject) -> String {
    match subject {
        Subject::NamedNode(node) => serialize_named_node(node),
        Subject::BlankNode(node) => format!("_:{}", node.label()),
    }
}

fn serialize_object(object: &Object) -> String {
    match object {
        Object::NamedNode(node) => serialize_named_node(node),
        Object::BlankNode(node) => format!("_:{}", node.label()),
        Object::Literal(literal) => serialize_literal(literal),
    }
}

fn serialize_named_node(node: &NamedNode) -> String {
    format!("<{}>", escape_iri(node.as_str()))
}

fn serialize_literal(literal: &Literal) -> String {
    let mut out = String::new();
    out.push('"');
    out.push_str(&escape_literal(literal.lexical_form()));
    out.push('"');
    if let Some(language) = literal.language() {
        out.push('@');
        out.push_str(language);
    } else if literal.datatype().as_str() != XSD_STRING_IRI {
        out.push_str("^^");
        out.push_str(&serialize_named_node(literal.datatype()));
    }
    out
}

fn escape_iri(iri: &str) -> String {
    let mut out = String::new();
    for ch in iri.chars() {
        if ch.is_control()
            || matches!(ch, '<' | '>' | '"' | '\\' | '{' | '}' | '|' | '^' | '`' | ' ')
        {
            out.push_str(&unicode_escape(ch));
        } else {
            out.push(ch);
        }
    }
    out
}

fn escape_literal(lexical: &str) -> String {
    let mut out = String::new();
    for ch in lexical.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000C}' => out.push_str("\\f"),
            c if c.is_control() => out.push_str(&unicode_escape(c)),
            _ => out.push(ch),
        }
    }
    out
}

fn unicode_escape(ch: char) -> String {
    if (ch as u32) <= 0xFFFF {
        format!("\\u{:04X}", ch as u32)
    } else {
        format!("\\U{:08X}", ch as u32)
    }
}

fn parse_ntriples_line(input: &str, line: usize) -> Result<Option<Triple>, NTriplesError> {
    let mut parser = LineParser::new(input, line);
    parser.parse_line()
}

fn parse_nquads_line(input: &str, line: usize) -> Result<Option<Quad>, NTriplesError> {
    let mut parser = LineParser::new(input, line);
    parser.parse_quad_line()
}

struct LineParser<'a> {
    input: &'a str,
    bytes: &'a [u8],
    line: usize,
    pos: usize,
}

impl<'a> LineParser<'a> {
    fn new(input: &'a str, line: usize) -> Self {
        Self { input, bytes: input.as_bytes(), line, pos: 0 }
    }

    fn parse_line(&mut self) -> Result<Option<Triple>, NTriplesError> {
        self.skip_ws();
        if self.is_eof() || self.peek() == Some(b'#') {
            return Ok(None);
        }

        let subject = self.parse_subject()?;
        self.expect_ws()?;
        let predicate =
            Predicate::new(self.parse_named_node(NTriplesErrorKind::ExpectedPredicate)?);
        self.expect_ws()?;
        let object = self.parse_object()?;
        self.skip_ws();
        self.expect_byte(b'.', NTriplesErrorKind::ExpectedDot, "expected '.' at end of triple")?;
        self.skip_ws();
        if self.peek() == Some(b'#') {
            self.pos = self.bytes.len();
        }
        if !self.is_eof() {
            return Err(self.error(
                NTriplesErrorKind::TrailingCharacters,
                "unexpected trailing characters after '.'",
            ));
        }
        Ok(Some(Triple::new(subject, predicate, object)))
    }

    fn parse_quad_line(&mut self) -> Result<Option<Quad>, NTriplesError> {
        self.skip_ws();
        if self.is_eof() || self.peek() == Some(b'#') {
            return Ok(None);
        }

        let subject = self.parse_subject()?;
        self.expect_ws()?;
        let predicate =
            Predicate::new(self.parse_named_node(NTriplesErrorKind::ExpectedPredicate)?);
        self.expect_ws()?;
        let object = self.parse_object()?;
        self.skip_ws();

        let graph_name = if self.peek() == Some(b'.') {
            GraphName::DefaultGraph
        } else {
            let graph = self.parse_graph_name()?;
            self.skip_ws();
            graph
        };

        self.expect_byte(b'.', NTriplesErrorKind::ExpectedDot, "expected '.' at end of quad")?;
        self.skip_ws();
        if self.peek() == Some(b'#') {
            self.pos = self.bytes.len();
        }
        if !self.is_eof() {
            return Err(self.error(
                NTriplesErrorKind::TrailingCharacters,
                "unexpected trailing characters after '.'",
            ));
        }
        Ok(Some(Quad::new(subject, predicate, object, graph_name)))
    }

    fn parse_subject(&mut self) -> Result<Subject, NTriplesError> {
        if self.peek() == Some(b'<') {
            return Ok(Subject::NamedNode(
                self.parse_named_node(NTriplesErrorKind::ExpectedSubject)?,
            ));
        }
        if self.starts_with("_:") {
            return Ok(Subject::BlankNode(self.parse_blank_node()?));
        }
        Err(self.error(NTriplesErrorKind::ExpectedSubject, "subject must be an IRI or blank node"))
    }

    fn parse_object(&mut self) -> Result<Object, NTriplesError> {
        match self.peek() {
            Some(b'<') => {
                Ok(Object::NamedNode(self.parse_named_node(NTriplesErrorKind::ExpectedObject)?))
            }
            Some(b'"') => Ok(Object::Literal(self.parse_literal()?)),
            _ if self.starts_with("_:") => Ok(Object::BlankNode(self.parse_blank_node()?)),
            _ => Err(self.error(
                NTriplesErrorKind::ExpectedObject,
                "object must be an IRI, blank node, or literal",
            )),
        }
    }

    fn parse_graph_name(&mut self) -> Result<GraphName, NTriplesError> {
        if self.peek() == Some(b'<') {
            return Ok(GraphName::NamedNode(
                self.parse_named_node(NTriplesErrorKind::ExpectedObject)?,
            ));
        }
        if self.starts_with("_:") {
            return Ok(GraphName::BlankNode(self.parse_blank_node()?));
        }
        Err(self
            .error(NTriplesErrorKind::ExpectedObject, "graph name must be an IRI or blank node"))
    }

    fn parse_named_node(
        &mut self,
        expected_kind: NTriplesErrorKind,
    ) -> Result<NamedNode, NTriplesError> {
        if self.peek() != Some(b'<') {
            return Err(self.error(expected_kind, "expected IRI in angle brackets"));
        }
        let iri = self.parse_iri_lexical()?;
        NamedNode::new(iri.clone())
            .map_err(|_| self.error(NTriplesErrorKind::InvalidIri, format!("invalid IRI: {iri}")))
    }

    fn parse_blank_node(&mut self) -> Result<BlankNode, NTriplesError> {
        if !self.starts_with("_:") {
            return Err(
                self.error(NTriplesErrorKind::InvalidBlankNode, "expected blank node label")
            );
        }
        self.pos += 2;
        let start = self.pos;
        let Some(first) = self.peek() else {
            return Err(
                self.error(NTriplesErrorKind::InvalidBlankNode, "blank node label is empty")
            );
        };
        if !((first as char).is_ascii_alphabetic() || first == b'_') {
            return Err(self.error(
                NTriplesErrorKind::InvalidBlankNode,
                "blank node must start with ASCII letter or '_'",
            ));
        }
        self.pos += 1;
        while let Some(byte) = self.peek() {
            let ch = byte as char;
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.') {
                self.pos += 1;
            } else {
                break;
            }
        }
        let label = &self.input[start..self.pos];
        BlankNode::new(label.to_string()).map_err(|_| {
            self.error(NTriplesErrorKind::InvalidBlankNode, "invalid blank node label")
        })
    }

    fn parse_literal(&mut self) -> Result<Literal, NTriplesError> {
        let lexical = self.parse_literal_lexical()?;
        if self.peek() == Some(b'@') {
            self.pos += 1;
            let tag_start = self.pos;
            while let Some(byte) = self.peek() {
                let ch = byte as char;
                if ch.is_ascii_alphanumeric() || ch == '-' {
                    self.pos += 1;
                } else {
                    break;
                }
            }
            let tag = &self.input[tag_start..self.pos];
            if tag.is_empty() {
                return Err(self.error(
                    NTriplesErrorKind::InvalidLanguageTag,
                    "expected language tag after '@'",
                ));
            }
            return Literal::new_language_tagged(lexical, tag)
                .map_err(|error| self.map_model_error(error));
        }
        if self.starts_with("^^") {
            self.pos += 2;
            let datatype = self.parse_named_node(NTriplesErrorKind::InvalidLiteral)?;
            return Literal::new_typed(lexical, datatype)
                .map_err(|error| self.map_model_error(error));
        }
        Ok(Literal::new_simple(lexical))
    }

    fn parse_literal_lexical(&mut self) -> Result<String, NTriplesError> {
        self.expect_byte(
            b'"',
            NTriplesErrorKind::InvalidLiteral,
            "expected '\"' to start literal",
        )?;
        let mut out = String::new();
        let mut segment_start = self.pos;
        loop {
            let Some(byte) = self.peek() else {
                return Err(
                    self.error(NTriplesErrorKind::UnexpectedEof, "unterminated literal string")
                );
            };
            match byte {
                b'"' => {
                    out.push_str(&self.input[segment_start..self.pos]);
                    self.pos += 1;
                    return Ok(out);
                }
                b'\\' => {
                    out.push_str(&self.input[segment_start..self.pos]);
                    let escaped = self.parse_literal_escape()?;
                    out.push(escaped);
                    segment_start = self.pos;
                }
                b'\n' | b'\r' => {
                    return Err(self.error(
                        NTriplesErrorKind::InvalidLiteral,
                        "literal must not contain raw newlines",
                    ));
                }
                value if value < 0x20 => {
                    return Err(self.error(
                        NTriplesErrorKind::InvalidLiteral,
                        "literal contains unescaped control character",
                    ));
                }
                _ => {
                    self.pos += 1;
                }
            }
        }
    }

    fn parse_literal_escape(&mut self) -> Result<char, NTriplesError> {
        if self.peek() != Some(b'\\') {
            return Err(self.error(
                NTriplesErrorKind::InvalidEscape,
                "expected '\\' to start escape sequence",
            ));
        }
        if self.pos + 1 >= self.bytes.len() {
            return Err(
                self.error(NTriplesErrorKind::UnexpectedEof, "escape sequence is truncated")
            );
        }
        let next = self.bytes[self.pos + 1];
        match next {
            b't' => {
                self.pos += 2;
                Ok('\t')
            }
            b'b' => {
                self.pos += 2;
                Ok('\u{0008}')
            }
            b'n' => {
                self.pos += 2;
                Ok('\n')
            }
            b'r' => {
                self.pos += 2;
                Ok('\r')
            }
            b'f' => {
                self.pos += 2;
                Ok('\u{000C}')
            }
            b'"' => {
                self.pos += 2;
                Ok('"')
            }
            b'\\' => {
                self.pos += 2;
                Ok('\\')
            }
            b'u' | b'U' => self.parse_unicode_escape(),
            _ => {
                Err(self
                    .error(NTriplesErrorKind::InvalidEscape, "unsupported literal escape sequence"))
            }
        }
    }

    fn parse_iri_lexical(&mut self) -> Result<String, NTriplesError> {
        self.expect_byte(b'<', NTriplesErrorKind::InvalidIri, "expected '<' to start IRI token")?;
        let mut out = String::new();
        let mut segment_start = self.pos;
        loop {
            let Some(byte) = self.peek() else {
                return Err(self.error(NTriplesErrorKind::UnexpectedEof, "unterminated IRI token"));
            };
            match byte {
                b'>' => {
                    out.push_str(&self.input[segment_start..self.pos]);
                    self.pos += 1;
                    return Ok(out);
                }
                b'\\' => {
                    out.push_str(&self.input[segment_start..self.pos]);
                    let escaped = self.parse_unicode_escape()?;
                    out.push(escaped);
                    segment_start = self.pos;
                }
                value if value <= 0x20 => {
                    return Err(self.error(
                        NTriplesErrorKind::InvalidIri,
                        "IRI contains disallowed whitespace/control character",
                    ));
                }
                _ => {
                    self.pos += 1;
                }
            }
        }
    }

    fn parse_unicode_escape(&mut self) -> Result<char, NTriplesError> {
        if self.peek() != Some(b'\\') {
            return Err(
                self.error(NTriplesErrorKind::InvalidEscape, "unicode escape must start with '\\'")
            );
        }
        if self.pos + 1 >= self.bytes.len() {
            return Err(self
                .error(NTriplesErrorKind::UnexpectedEof, "unicode escape sequence is truncated"));
        }
        let marker = self.bytes[self.pos + 1];
        let digits = match marker {
            b'u' => 4,
            b'U' => 8,
            _ => {
                return Err(self.error(
                    NTriplesErrorKind::InvalidEscape,
                    "unicode escape must use \\uXXXX or \\UXXXXXXXX",
                ));
            }
        };
        let hex_start = self.pos + 2;
        let hex_end = hex_start + digits;
        if hex_end > self.bytes.len() {
            return Err(self
                .error(NTriplesErrorKind::UnexpectedEof, "unicode escape sequence is truncated"));
        }
        let hex = &self.input[hex_start..hex_end];
        if !hex.as_bytes().iter().all(u8::is_ascii_hexdigit) {
            return Err(self.error(
                NTriplesErrorKind::InvalidEscape,
                "unicode escape contains non-hex characters",
            ));
        }
        let code_point = u32::from_str_radix(hex, 16).map_err(|_| {
            self.error(
                NTriplesErrorKind::InvalidEscape,
                "unicode escape contains invalid code point",
            )
        })?;
        let decoded = char::from_u32(code_point).ok_or_else(|| {
            self.error(
                NTriplesErrorKind::InvalidEscape,
                "unicode escape decoded to invalid scalar value",
            )
        })?;
        self.pos = hex_end;
        Ok(decoded)
    }

    fn map_model_error(&self, error: ModelError) -> NTriplesError {
        match error {
            ModelError::InvalidLanguageTag(tag) => self.error(
                NTriplesErrorKind::InvalidLanguageTag,
                format!("invalid language tag: {tag}"),
            ),
            ModelError::MissingLanguageTag => self.error(
                NTriplesErrorKind::InvalidLanguageTag,
                "rdf:langString requires a language tag",
            ),
            ModelError::InvalidIri(iri) => {
                self.error(NTriplesErrorKind::InvalidIri, format!("invalid IRI: {iri}"))
            }
            ModelError::InvalidBlankNodeLabel(label) => self.error(
                NTriplesErrorKind::InvalidBlankNode,
                format!("invalid blank node label: {label}"),
            ),
        }
    }

    fn expect_ws(&mut self) -> Result<(), NTriplesError> {
        let start = self.pos;
        self.skip_ws();
        if self.pos == start {
            return Err(self.error(
                NTriplesErrorKind::ExpectedWhitespace,
                "expected at least one whitespace separator",
            ));
        }
        Ok(())
    }

    fn expect_byte(
        &mut self,
        expected: u8,
        kind: NTriplesErrorKind,
        message: &str,
    ) -> Result<(), NTriplesError> {
        if self.peek() != Some(expected) {
            return Err(self.error(kind, message));
        }
        self.pos += 1;
        Ok(())
    }

    fn starts_with(&self, text: &str) -> bool {
        self.input[self.pos..].starts_with(text)
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t')) {
            self.pos += 1;
        }
    }

    fn is_eof(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn error(&self, kind: NTriplesErrorKind, message: impl Into<String>) -> NTriplesError {
        NTriplesError::new(kind, self.line, self.pos + 1, message)
    }
}

pub type NQuadsErrorKind = NTriplesErrorKind;
pub type NQuadsError = NTriplesError;

pub struct NQuadsParser<R: BufRead> {
    lines: io::Lines<R>,
    line_number: usize,
}

impl<R: BufRead> Iterator for NQuadsParser<R> {
    type Item = Result<Quad, NQuadsError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let raw_line = self.lines.next()?;
            self.line_number += 1;
            match raw_line {
                Ok(line) => match parse_nquads_line(&line, self.line_number) {
                    Ok(Some(quad)) => return Some(Ok(quad)),
                    Ok(None) => {}
                    Err(error) => return Some(Err(error)),
                },
                Err(error) => {
                    return Some(Err(NTriplesError::new(
                        NTriplesErrorKind::Io,
                        self.line_number,
                        1,
                        error.to_string(),
                    )));
                }
            }
        }
    }
}

pub fn parse_nquads_reader<R: BufRead>(reader: R) -> NQuadsParser<R> {
    NQuadsParser { lines: reader.lines(), line_number: 0 }
}

pub fn parse_nquads(input: &str) -> Result<Vec<Quad>, NQuadsError> {
    parse_nquads_reader(Cursor::new(input.as_bytes())).collect()
}

pub fn serialize_nquads<W: Write>(writer: &mut W, quads: &[Quad]) -> io::Result<()> {
    for quad in quads {
        let graph = match quad.graph_name() {
            GraphName::DefaultGraph => String::new(),
            GraphName::NamedNode(node) => format!(" {}", serialize_named_node(node)),
            GraphName::BlankNode(node) => format!(" _:{label}", label = node.label()),
        };
        writeln!(
            writer,
            "{} {} {}{} .",
            serialize_subject(quad.subject()),
            serialize_named_node(quad.predicate().as_named_node()),
            serialize_object(quad.object()),
            graph
        )?;
    }
    Ok(())
}

pub fn serialize_nquads_to_string(quads: &[Quad]) -> io::Result<String> {
    let mut buffer = Vec::new();
    serialize_nquads(&mut buffer, quads)?;
    Ok(String::from_utf8(buffer).expect("serializer emitted invalid UTF-8"))
}

const RDF_TYPE_IRI: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum TurtleErrorKind {
    #[error("I/O error")]
    Io,
    #[error("lexing error")]
    LexError,
    #[error("unexpected token")]
    UnexpectedToken,
    #[error("unexpected end of input")]
    UnexpectedEof,
    #[error("unknown prefix")]
    UnknownPrefix,
    #[error("invalid IRI")]
    InvalidIri,
    #[error("invalid blank node")]
    InvalidBlankNode,
    #[error("invalid literal")]
    InvalidLiteral,
    #[error("invalid language tag")]
    InvalidLanguageTag,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("{kind} at line {line}, column {column}: {message}")]
pub struct TurtleError {
    pub kind: TurtleErrorKind,
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl TurtleError {
    fn new(kind: TurtleErrorKind, line: usize, column: usize, message: impl Into<String>) -> Self {
        Self { kind, line, column, message: message.into() }
    }
}

pub type TrigErrorKind = TurtleErrorKind;
pub type TrigError = TurtleError;

#[derive(Clone, Debug, PartialEq, Eq)]
struct TurtleToken {
    kind: TurtleTokenKind,
    lexeme: String,
    line: usize,
    column: usize,
}

#[derive(Logos, Clone, Debug, PartialEq, Eq)]
enum TurtleTokenKind {
    #[regex(r"[ \t\r\n\f]+", logos::skip)]
    #[regex(r"#([^\n\r]*)", logos::skip)]
    Ignored,
    #[token("@prefix")]
    PrefixDecl,
    #[token("^^")]
    TypeMarker,
    #[regex(r"@[A-Za-z]+(-[A-Za-z0-9]{1,8})*")]
    LangTag,
    #[token("{")]
    LBrace,
    #[token("}")]
    RBrace,
    #[token(".")]
    Dot,
    #[token("a")]
    A,
    #[regex(r"_:[A-Za-z_][A-Za-z0-9_.-]*")]
    BlankNode,
    #[regex(r"<[^>]*>")]
    IriRef,
    #[regex(r"[A-Za-z][A-Za-z0-9_-]*:[A-Za-z][A-Za-z0-9._-]*|:[A-Za-z][A-Za-z0-9._-]*")]
    PrefixedName,
    #[regex(r"[A-Za-z][A-Za-z0-9_-]*:|:")]
    PrefixNs,
    #[regex(r#""([^"\\]|\\.)*""#)]
    StringLiteral,
}

fn tokenize_turtle(input: &str) -> Result<Vec<TurtleToken>, TurtleError> {
    let mut lexer = TurtleTokenKind::lexer(input);
    let mut tokens = Vec::new();
    while let Some(kind) = lexer.next() {
        let span = lexer.span();
        let (line, column) = byte_index_to_line_column(input, span.start);
        match kind {
            Ok(TurtleTokenKind::Ignored) => {}
            Err(()) => {
                return Err(TurtleError::new(
                    TurtleErrorKind::LexError,
                    line,
                    column,
                    "invalid token",
                ));
            }
            Ok(kind) => {
                tokens.push(TurtleToken { kind, lexeme: input[span].to_string(), line, column });
            }
        }
    }
    Ok(tokens)
}

pub fn parse_turtle_reader<R: BufRead>(mut reader: R) -> Result<Vec<Quad>, TurtleError> {
    let mut source = String::new();
    reader.read_to_string(&mut source).map_err(|error| {
        TurtleError::new(TurtleErrorKind::Io, 1, 1, format!("failed to read input: {error}"))
    })?;
    parse_turtle(&source)
}

pub fn parse_turtle(input: &str) -> Result<Vec<Quad>, TurtleError> {
    let tokens = tokenize_turtle(input)?;
    let mut parser = TurtleParser::new(tokens);
    parser.parse_document(false)
}

pub fn parse_trig_reader<R: BufRead>(mut reader: R) -> Result<Vec<Quad>, TrigError> {
    let mut source = String::new();
    reader.read_to_string(&mut source).map_err(|error| {
        TurtleError::new(TurtleErrorKind::Io, 1, 1, format!("failed to read input: {error}"))
    })?;
    parse_trig(&source)
}

pub fn parse_trig(input: &str) -> Result<Vec<Quad>, TrigError> {
    let tokens = tokenize_turtle(input)?;
    let mut parser = TurtleParser::new(tokens);
    parser.parse_document(true)
}

pub fn serialize_turtle<W: Write>(writer: &mut W, quads: &[Quad]) -> io::Result<()> {
    for quad in quads {
        if quad.graph_name() != &GraphName::DefaultGraph {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "turtle serializer only accepts default-graph quads",
            ));
        }
        writeln!(
            writer,
            "{} {} {} .",
            serialize_subject(quad.subject()),
            serialize_named_node(quad.predicate().as_named_node()),
            serialize_object(quad.object())
        )?;
    }
    Ok(())
}

pub fn serialize_turtle_to_string(quads: &[Quad]) -> io::Result<String> {
    let mut buffer = Vec::new();
    serialize_turtle(&mut buffer, quads)?;
    Ok(String::from_utf8(buffer).expect("serializer emitted invalid UTF-8"))
}

pub fn serialize_trig<W: Write>(writer: &mut W, quads: &[Quad]) -> io::Result<()> {
    let mut default_graph = Vec::new();
    let mut named_graphs: Vec<(String, Vec<&Quad>)> = Vec::new();

    for quad in quads {
        if quad.graph_name() == &GraphName::DefaultGraph {
            default_graph.push(quad);
            continue;
        }
        let graph_lexeme = match quad.graph_name() {
            GraphName::NamedNode(node) => serialize_named_node(node),
            GraphName::BlankNode(node) => format!("_:{}", node.label()),
            GraphName::DefaultGraph => String::new(),
        };
        if let Some((_, grouped)) = named_graphs.iter_mut().find(|(name, _)| name == &graph_lexeme)
        {
            grouped.push(quad);
        } else {
            named_graphs.push((graph_lexeme, vec![quad]));
        }
    }

    for quad in default_graph {
        writeln!(
            writer,
            "{} {} {} .",
            serialize_subject(quad.subject()),
            serialize_named_node(quad.predicate().as_named_node()),
            serialize_object(quad.object())
        )?;
    }

    for (graph_lexeme, graph_quads) in named_graphs {
        writeln!(writer, "{graph_lexeme} {{")?;
        for quad in graph_quads {
            writeln!(
                writer,
                "  {} {} {} .",
                serialize_subject(quad.subject()),
                serialize_named_node(quad.predicate().as_named_node()),
                serialize_object(quad.object())
            )?;
        }
        writeln!(writer, "}}")?;
    }

    Ok(())
}

pub fn serialize_trig_to_string(quads: &[Quad]) -> io::Result<String> {
    let mut buffer = Vec::new();
    serialize_trig(&mut buffer, quads)?;
    Ok(String::from_utf8(buffer).expect("serializer emitted invalid UTF-8"))
}

struct TurtleParser {
    tokens: Vec<TurtleToken>,
    pos: usize,
    prefixes: HashMap<String, String>,
}

impl TurtleParser {
    fn new(tokens: Vec<TurtleToken>) -> Self {
        Self { tokens, pos: 0, prefixes: HashMap::new() }
    }

    fn parse_document(&mut self, allow_graph_blocks: bool) -> Result<Vec<Quad>, TurtleError> {
        let mut quads = Vec::new();
        while !self.is_eof() {
            if self.matches(TurtleTokenKind::PrefixDecl) {
                self.parse_prefix_decl()?;
                continue;
            }

            if allow_graph_blocks && self.matches(TurtleTokenKind::LBrace) {
                self.advance();
                self.parse_graph_block(&mut quads, GraphName::DefaultGraph)?;
                continue;
            }

            if allow_graph_blocks
                && matches!(
                    self.peek_kind(),
                    Some(
                        TurtleTokenKind::IriRef
                            | TurtleTokenKind::PrefixedName
                            | TurtleTokenKind::BlankNode
                    )
                )
                && self.peek_n_kind(1) == Some(TurtleTokenKind::LBrace)
            {
                let graph_name = self.parse_graph_name_token()?;
                self.expect(TurtleTokenKind::LBrace, "expected '{' after graph name")?;
                self.parse_graph_block(&mut quads, graph_name)?;
                continue;
            }

            let quad = self.parse_triple_statement(GraphName::DefaultGraph)?;
            quads.push(quad);
        }
        Ok(quads)
    }

    fn parse_graph_block(
        &mut self,
        output: &mut Vec<Quad>,
        graph_name: GraphName,
    ) -> Result<(), TurtleError> {
        while !self.is_eof() && !self.matches(TurtleTokenKind::RBrace) {
            output.push(self.parse_triple_statement(graph_name.clone())?);
        }
        self.expect(TurtleTokenKind::RBrace, "expected '}' to close graph block")?;
        if self.matches(TurtleTokenKind::Dot) {
            self.advance();
        }
        Ok(())
    }

    fn parse_prefix_decl(&mut self) -> Result<(), TurtleError> {
        self.expect(TurtleTokenKind::PrefixDecl, "expected '@prefix'")?;
        let ns_token = self.expect(TurtleTokenKind::PrefixNs, "expected prefix namespace")?;
        let prefix_name = parse_prefix_ns(&ns_token.lexeme).map_err(|message| {
            TurtleError::new(
                TurtleErrorKind::UnexpectedToken,
                ns_token.line,
                ns_token.column,
                message,
            )
        })?;
        let iri_token = self.expect(TurtleTokenKind::IriRef, "expected IRI after @prefix")?;
        let iri = parse_iri_token(&iri_token)?;
        self.expect(TurtleTokenKind::Dot, "expected '.' after prefix declaration")?;
        self.prefixes.insert(prefix_name, iri.as_str().to_string());
        Ok(())
    }

    fn parse_triple_statement(&mut self, graph_name: GraphName) -> Result<Quad, TurtleError> {
        let subject = self.parse_subject_token()?;
        let predicate = self.parse_predicate_token()?;
        let object = self.parse_object_token()?;
        self.expect(TurtleTokenKind::Dot, "expected '.' after triple statement")?;
        Ok(Quad::new(subject, predicate, object, graph_name))
    }

    fn parse_subject_token(&mut self) -> Result<Subject, TurtleError> {
        let token = self.peek().ok_or_else(|| {
            TurtleError::new(TurtleErrorKind::UnexpectedEof, 1, 1, "expected subject token")
        })?;
        match token.kind {
            TurtleTokenKind::IriRef | TurtleTokenKind::PrefixedName => {
                Ok(Subject::NamedNode(self.parse_named_node_token()?))
            }
            TurtleTokenKind::BlankNode => {
                let bnode = parse_blank_node_token(&self.advance().expect("peeked token"))?;
                Ok(Subject::BlankNode(bnode))
            }
            _ => Err(TurtleError::new(
                TurtleErrorKind::UnexpectedToken,
                token.line,
                token.column,
                "expected subject token",
            )),
        }
    }

    fn parse_predicate_token(&mut self) -> Result<Predicate, TurtleError> {
        let token = self.peek().ok_or_else(|| {
            TurtleError::new(TurtleErrorKind::UnexpectedEof, 1, 1, "expected predicate token")
        })?;
        if token.kind == TurtleTokenKind::A {
            let a = NamedNode::new(RDF_TYPE_IRI.to_string()).map_err(|_| {
                TurtleError::new(TurtleErrorKind::InvalidIri, token.line, token.column, "rdf:type")
            })?;
            self.advance();
            return Ok(Predicate::new(a));
        }
        Ok(Predicate::new(self.parse_named_node_token()?))
    }

    fn parse_object_token(&mut self) -> Result<Object, TurtleError> {
        let token = self.peek().ok_or_else(|| {
            TurtleError::new(TurtleErrorKind::UnexpectedEof, 1, 1, "expected object token")
        })?;
        match token.kind {
            TurtleTokenKind::IriRef | TurtleTokenKind::PrefixedName => {
                Ok(Object::NamedNode(self.parse_named_node_token()?))
            }
            TurtleTokenKind::BlankNode => {
                let bnode = parse_blank_node_token(&self.advance().expect("peeked token"))?;
                Ok(Object::BlankNode(bnode))
            }
            TurtleTokenKind::StringLiteral => Ok(Object::Literal(self.parse_literal_token()?)),
            _ => Err(TurtleError::new(
                TurtleErrorKind::UnexpectedToken,
                token.line,
                token.column,
                "expected object token",
            )),
        }
    }

    fn parse_graph_name_token(&mut self) -> Result<GraphName, TurtleError> {
        let token = self.peek().ok_or_else(|| {
            TurtleError::new(TurtleErrorKind::UnexpectedEof, 1, 1, "expected graph name token")
        })?;
        match token.kind {
            TurtleTokenKind::IriRef | TurtleTokenKind::PrefixedName => {
                Ok(GraphName::NamedNode(self.parse_named_node_token()?))
            }
            TurtleTokenKind::BlankNode => {
                let bnode = parse_blank_node_token(&self.advance().expect("peeked token"))?;
                Ok(GraphName::BlankNode(bnode))
            }
            _ => Err(TurtleError::new(
                TurtleErrorKind::UnexpectedToken,
                token.line,
                token.column,
                "expected graph name token",
            )),
        }
    }

    fn parse_named_node_token(&mut self) -> Result<NamedNode, TurtleError> {
        let token = self.advance().ok_or_else(|| {
            TurtleError::new(TurtleErrorKind::UnexpectedEof, 1, 1, "expected named node token")
        })?;
        match token.kind {
            TurtleTokenKind::IriRef => parse_iri_token(&token),
            TurtleTokenKind::PrefixedName => {
                let (prefix, local) = parse_prefixed_name(&token.lexeme).map_err(|message| {
                    TurtleError::new(
                        TurtleErrorKind::UnexpectedToken,
                        token.line,
                        token.column,
                        message,
                    )
                })?;
                let namespace = self.prefixes.get(&prefix).ok_or_else(|| {
                    TurtleError::new(
                        TurtleErrorKind::UnknownPrefix,
                        token.line,
                        token.column,
                        format!("unknown prefix '{prefix}:'"),
                    )
                })?;
                let iri = format!("{namespace}{local}");
                NamedNode::new(iri).map_err(|_| {
                    TurtleError::new(
                        TurtleErrorKind::InvalidIri,
                        token.line,
                        token.column,
                        "invalid prefixed-name expansion",
                    )
                })
            }
            _ => Err(TurtleError::new(
                TurtleErrorKind::UnexpectedToken,
                token.line,
                token.column,
                "expected named node token",
            )),
        }
    }

    fn parse_literal_token(&mut self) -> Result<Literal, TurtleError> {
        let token = self.expect(TurtleTokenKind::StringLiteral, "expected string literal token")?;
        let lexical = parse_literal_token(&token)?;
        if self.matches(TurtleTokenKind::LangTag) {
            let lang = self.advance().expect("matched lang token");
            let tag = lang.lexeme.trim_start_matches('@');
            return Literal::new_language_tagged(lexical, tag).map_err(|_| {
                TurtleError::new(
                    TurtleErrorKind::InvalidLanguageTag,
                    lang.line,
                    lang.column,
                    format!("invalid language tag '{tag}'"),
                )
            });
        }
        if self.matches(TurtleTokenKind::TypeMarker) {
            self.advance();
            let datatype = self.parse_named_node_token()?;
            return Literal::new_typed(lexical, datatype).map_err(|_| {
                TurtleError::new(
                    TurtleErrorKind::InvalidLiteral,
                    token.line,
                    token.column,
                    "invalid typed literal",
                )
            });
        }
        Ok(Literal::new_simple(lexical))
    }

    fn expect(&mut self, kind: TurtleTokenKind, message: &str) -> Result<TurtleToken, TurtleError> {
        let token = self.advance().ok_or_else(|| {
            TurtleError::new(TurtleErrorKind::UnexpectedEof, 1, 1, message.to_string())
        })?;
        if token.kind != kind {
            return Err(TurtleError::new(
                TurtleErrorKind::UnexpectedToken,
                token.line,
                token.column,
                message.to_string(),
            ));
        }
        Ok(token)
    }

    fn matches(&self, kind: TurtleTokenKind) -> bool {
        self.peek_kind() == Some(kind)
    }

    fn peek_kind(&self) -> Option<TurtleTokenKind> {
        self.tokens.get(self.pos).map(|token| token.kind.clone())
    }

    fn peek_n_kind(&self, offset: usize) -> Option<TurtleTokenKind> {
        self.tokens.get(self.pos + offset).map(|token| token.kind.clone())
    }

    fn peek(&self) -> Option<&TurtleToken> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) -> Option<TurtleToken> {
        let token = self.tokens.get(self.pos)?.clone();
        self.pos += 1;
        Some(token)
    }

    fn is_eof(&self) -> bool {
        self.pos >= self.tokens.len()
    }
}

fn parse_prefix_ns(value: &str) -> Result<String, String> {
    let (prefix, rest) =
        value.split_once(':').ok_or_else(|| format!("invalid prefix namespace token '{value}'"))?;
    if !rest.is_empty() {
        return Err(format!("invalid prefix namespace token '{value}'"));
    }
    validate_with_winnow(prefix, true, |c| c.is_ascii_alphanumeric() || c == '_' || c == '-')?;
    Ok(prefix.to_string())
}

fn parse_prefixed_name(value: &str) -> Result<(String, String), String> {
    let (prefix, local) =
        value.split_once(':').ok_or_else(|| format!("invalid prefixed-name token '{value}'"))?;
    validate_with_winnow(prefix, true, |c| c.is_ascii_alphanumeric() || c == '_' || c == '-')?;
    validate_with_winnow(local, false, |c| {
        c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.'
    })?;
    Ok((prefix.to_string(), local.to_string()))
}

fn validate_with_winnow(
    input: &str,
    allow_empty: bool,
    predicate: impl Fn(char) -> bool + Copy,
) -> Result<(), String> {
    use winnow::Parser;
    use winnow::error::ContextError;
    use winnow::token::take_while;

    let mut text = input;
    let lower = if allow_empty { 0 } else { 1 };
    let _matched: &str = take_while::<_, _, ContextError>(lower.., predicate)
        .parse_next(&mut text)
        .map_err(|_| format!("invalid token segment '{input}'"))?;
    if !text.is_empty() {
        return Err(format!("invalid token segment '{input}'"));
    }
    Ok(())
}

fn parse_iri_token(token: &TurtleToken) -> Result<NamedNode, TurtleError> {
    let mut iri = token.lexeme.clone();
    iri.remove(0);
    iri.pop();
    let decoded = decode_escape_sequences(&iri, false, token.line, token.column)?;
    NamedNode::new(decoded).map_err(|_| {
        TurtleError::new(TurtleErrorKind::InvalidIri, token.line, token.column, "invalid IRI token")
    })
}

fn parse_blank_node_token(token: &TurtleToken) -> Result<BlankNode, TurtleError> {
    let label = token.lexeme.strip_prefix("_:").ok_or_else(|| {
        TurtleError::new(
            TurtleErrorKind::InvalidBlankNode,
            token.line,
            token.column,
            "blank node token missing '_:' prefix",
        )
    })?;
    BlankNode::new(label.to_string()).map_err(|_| {
        TurtleError::new(
            TurtleErrorKind::InvalidBlankNode,
            token.line,
            token.column,
            "invalid blank node label",
        )
    })
}

fn parse_literal_token(token: &TurtleToken) -> Result<String, TurtleError> {
    if token.lexeme.len() < 2 {
        return Err(TurtleError::new(
            TurtleErrorKind::InvalidLiteral,
            token.line,
            token.column,
            "literal token too short",
        ));
    }
    let lexical = &token.lexeme[1..token.lexeme.len() - 1];
    decode_escape_sequences(lexical, true, token.line, token.column)
}

fn decode_escape_sequences(
    input: &str,
    allow_basic: bool,
    line: usize,
    column: usize,
) -> Result<String, TurtleError> {
    let bytes = input.as_bytes();
    let mut pos = 0;
    let mut output = String::new();
    while pos < bytes.len() {
        if bytes[pos] != b'\\' {
            output.push(bytes[pos] as char);
            pos += 1;
            continue;
        }
        pos += 1;
        if pos >= bytes.len() {
            return Err(TurtleError::new(
                TurtleErrorKind::InvalidLiteral,
                line,
                column,
                "truncated escape sequence",
            ));
        }
        match bytes[pos] {
            b't' if allow_basic => {
                output.push('\t');
                pos += 1;
            }
            b'b' if allow_basic => {
                output.push('\u{0008}');
                pos += 1;
            }
            b'n' if allow_basic => {
                output.push('\n');
                pos += 1;
            }
            b'r' if allow_basic => {
                output.push('\r');
                pos += 1;
            }
            b'f' if allow_basic => {
                output.push('\u{000C}');
                pos += 1;
            }
            b'"' if allow_basic => {
                output.push('"');
                pos += 1;
            }
            b'\'' if allow_basic => {
                output.push('\'');
                pos += 1;
            }
            b'\\' => {
                output.push('\\');
                pos += 1;
            }
            b'u' | b'U' => {
                let digits = if bytes[pos] == b'u' { 4 } else { 8 };
                pos += 1;
                if pos + digits > bytes.len() {
                    return Err(TurtleError::new(
                        TurtleErrorKind::InvalidLiteral,
                        line,
                        column,
                        "truncated unicode escape",
                    ));
                }
                let hex = &input[pos..pos + digits];
                if !hex.as_bytes().iter().all(u8::is_ascii_hexdigit) {
                    return Err(TurtleError::new(
                        TurtleErrorKind::InvalidLiteral,
                        line,
                        column,
                        "unicode escape contains non-hex digits",
                    ));
                }
                let codepoint = u32::from_str_radix(hex, 16).map_err(|_| {
                    TurtleError::new(
                        TurtleErrorKind::InvalidLiteral,
                        line,
                        column,
                        "invalid unicode code point",
                    )
                })?;
                let decoded = char::from_u32(codepoint).ok_or_else(|| {
                    TurtleError::new(
                        TurtleErrorKind::InvalidLiteral,
                        line,
                        column,
                        "unicode escape decoded to invalid scalar",
                    )
                })?;
                output.push(decoded);
                pos += digits;
            }
            _ => {
                return Err(TurtleError::new(
                    TurtleErrorKind::InvalidLiteral,
                    line,
                    column,
                    "unsupported escape sequence",
                ));
            }
        }
    }
    Ok(output)
}

fn byte_index_to_line_column(input: &str, index: usize) -> (usize, usize) {
    let mut line = 1;
    let mut column = 1;
    for &byte in input.as_bytes().iter().take(index) {
        if byte == b'\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    (line, column)
}

#[cfg(test)]
mod tests {
    use super::{NTriplesErrorKind, parse_ntriples};

    #[test]
    fn parses_single_line() {
        let parsed = parse_ntriples("<http://example.com/s> <http://example.com/p> \"o\" .\n")
            .expect("parse line");
        assert_eq!(parsed.len(), 1);
    }

    #[test]
    fn reports_missing_dot() {
        let error = parse_ntriples("<http://example.com/s> <http://example.com/p> \"o\"")
            .expect_err("missing dot");
        assert_eq!(error.kind, NTriplesErrorKind::ExpectedDot);
        assert_eq!(error.line, 1);
    }
}
