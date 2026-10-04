//! One representative source and reviewed token-range oracle per grammar.
//! Role names are Jaunder's closed semantic CSS categories, not query names.

pub(super) struct Case {
    pub label: &'static str,
    pub code: &'static str,
    pub roles: &'static [(&'static str, &'static str)],
}

pub(super) const CASES: &[Case] = &[
    Case {
        label: "asm",
        code: "mov eax, 42\n; a comment\n",
        roles: &[("42", "number"), ("; a comment", "comment")],
    },
    Case {
        label: "bash",
        code: "name=\"hello\"\necho \"$name\" # note\n",
        roles: &[("\"hello\"", "string"), ("# note", "comment")],
    },
    Case {
        label: "c",
        code: "int answer(void) { return 42; }\n",
        roles: &[("return", "keyword"), ("42", "number")],
    },
    Case {
        label: "c-sharp",
        code: "class Hello { string Say() { return \"hello\"; } }\n",
        roles: &[("class", "keyword"), ("\"hello\"", "string")],
    },
    Case {
        label: "cmake",
        code: "cmake_minimum_required(VERSION 3.20)\nmessage(\"hello\")\n",
        roles: &[("message", "function"), ("\"hello\"", "string")],
    },
    Case {
        label: "dockerfile",
        code: "FROM alpine:3.20\nRUN echo \"hello\"\nCMD [\"echo\", \"hello\"]\n",
        roles: &[("FROM", "keyword"), ("\"hello\"", "string")],
    },
    Case {
        label: "cpp",
        code: "int answer() { return 42; }\n",
        roles: &[("return", "keyword"), ("42", "number")],
    },
    Case {
        label: "css",
        code: "h1 { color: red; margin: 2px; }\n",
        roles: &[("color", "variable"), ("2px", "number")],
    },
    Case {
        label: "dart",
        code: "void main() { print(\"hello\"); }\n",
        roles: &[("void", "type"), ("\"hello\"", "string")],
    },
    Case {
        label: "diff",
        code: "--- a/note\n+++ b/note\n@@ -1 +1 @@\n-old\n+new\n",
        roles: &[("-old", "diff-minus"), ("+new", "diff-plus")],
    },
    Case {
        label: "elixir",
        code: "defmodule Hello do\n  def greet, do: \"hello\"\nend\n",
        roles: &[("defmodule", "keyword"), ("\"hello\"", "string")],
    },
    Case {
        label: "fish",
        code: "set -l name \"hello\"\necho $name\n",
        roles: &[("set", "function"), ("\"hello\"", "string")],
    },
    Case {
        label: "gleam",
        code: "pub fn greet(name: String) { \"hello \" <> name }\n",
        roles: &[("pub", "keyword"), ("\"hello \"", "string")],
    },
    Case {
        label: "go",
        code: "package main\nfunc main() { println(\"hello\") }\n",
        roles: &[("func", "keyword"), ("\"hello\"", "string")],
    },
    Case {
        label: "html",
        code: "<div class=\"hi\">hello</div>\n",
        roles: &[("div", "keyword"), ("\"hi\"", "string")],
    },
    Case {
        label: "java",
        code: "class Hello { String say() { return \"hello\"; } }\n",
        roles: &[("class", "keyword"), ("\"hello\"", "string")],
    },
    Case {
        label: "javascript",
        code: "const greet = (name) => \"hello\" + name;\n",
        roles: &[("const", "keyword"), ("\"hello\"", "string")],
    },
    Case {
        label: "json",
        code: "{\"message\": \"hello\", \"count\": 2}\n",
        roles: &[("\"hello\"", "string"), ("2", "number")],
    },
    Case {
        label: "julia",
        code: "function greet(name)\n  println(\"hello\")\nend\n",
        roles: &[("function", "keyword"), ("\"hello\"", "string")],
    },
    Case {
        label: "kotlin",
        code: "fun main() { println(\"hello\") }\n",
        roles: &[("fun", "keyword"), ("\"hello\"", "string")],
    },
    Case {
        label: "lua",
        code: "local function greet(name) return \"hello\" end\n",
        roles: &[("local", "keyword"), ("\"hello\"", "string")],
    },
    Case {
        label: "make",
        code: "MESSAGE = hello\nall:\n\t@echo $(MESSAGE)\n",
        roles: &[("all", "function"), ("MESSAGE", "string")],
    },
    Case {
        label: "markdown",
        code: "# Heading\n\n> quoted text\n",
        roles: &[("Heading", "heading"), ("quoted text", "quote")],
    },
    Case {
        label: "nix",
        code: "let name = \"hello\"; in name\n",
        roles: &[("let", "keyword"), ("\"hello\"", "string")],
    },
    Case {
        label: "ocaml",
        code: "let greet name = print_endline \"hello\"\n",
        roles: &[("let", "keyword"), ("\"hello\"", "string")],
    },
    Case {
        label: "php",
        code: "function greet($name) { return \"hello\"; }\n",
        roles: &[("function", "keyword"), ("\"hello\"", "string")],
    },
    Case {
        label: "python",
        code: "def greet(name):\n    return \"hello\"\n",
        roles: &[("def", "keyword"), ("\"hello\"", "string")],
    },
    Case {
        label: "r",
        code: "greet <- function(name) paste(\"hello\", name)\n",
        roles: &[("function", "keyword"), ("\"hello\"", "string")],
    },
    Case {
        label: "ql",
        code: "import java\nfrom Method m\nselect m\n",
        roles: &[("import", "keyword"), ("Method", "type")],
    },
    Case {
        label: "ruby",
        code: "def greet(name)\n  puts \"hello\"\nend\n",
        roles: &[("def", "keyword"), ("\"hello\"", "string")],
    },
    Case {
        label: "rust",
        code: "fn greet() { println!(\"hello\"); }\n",
        roles: &[("fn", "keyword"), ("\"hello\"", "string")],
    },
    Case {
        label: "scala",
        code: "object Hello { def greet = println(\"hello\") }\n",
        roles: &[("object", "keyword"), ("\"hello\"", "string")],
    },
    Case {
        label: "sql",
        code: "SELECT name FROM posts WHERE id = 42;\n",
        roles: &[("SELECT", "keyword"), ("42", "number")],
    },
    Case {
        label: "swift",
        code: "func greet() { print(\"hello\") }\n",
        roles: &[("func", "keyword"), ("\"hello\"", "string")],
    },
    Case {
        label: "toml",
        code: "title = \"hello\"\ncount = 2\n",
        roles: &[("\"hello\"", "string"), ("2", "number")],
    },
    Case {
        label: "typescript",
        code: "const greet = (name: string): string => \"hello\" + name;\n",
        roles: &[("const", "keyword"), ("\"hello\"", "string")],
    },
    Case {
        label: "tsx",
        code: "const Greeting = () => <h1>hello</h1>;\n",
        roles: &[("const", "keyword"), ("h1", "keyword")],
    },
    Case {
        label: "xml",
        code: "<root id=\"hi\"><item>hello</item></root>\n",
        roles: &[("root", "keyword"), ("\"hi\"", "string")],
    },
    Case {
        label: "yaml",
        code: "title: hello\ncount: 2\n",
        roles: &[("title", "variable"), ("2", "number")],
    },
    Case {
        label: "zig",
        code: "const std = @import(\"std\");\npub fn main() void { std.debug.print(\"hello\", .{}); }\n",
        roles: &[("const", "keyword"), ("\"hello\"", "string")],
    },
    Case {
        label: "elisp",
        code: super::ELISP,
        roles: &[("add-to-list", "function-call"), ("\"melpa\"", "string")],
    },
    Case {
        label: "haskell",
        code: "{-# LANGUAGE OverloadedStrings #-}\nmodule Main where\nmain = putStrLn \"hello\"\n",
        roles: &[("module", "keyword"), ("\"hello\"", "string")],
    },
];
