//! One representative source and reviewed token-range oracle per grammar.
//! Ranges use UTF-8 byte offsets into the exact fixture, not query names.

#[derive(Clone, Copy)]
pub(super) enum Role {
    Comment,
    Keyword,
    String,
    Number,
    Function,
    FunctionCall,
    DiffPlus,
    DiffMinus,
    Heading,
    Quote,
    Type,
    Variable,
    Punctuation,
}

impl Role {
    pub(super) fn class(self) -> &'static str {
        match self {
            Self::Comment => "comment",
            Self::Keyword => "keyword",
            Self::String => "string",
            Self::Number => "number",
            Self::Function => "function",
            Self::FunctionCall => "function-call",
            Self::DiffPlus => "diff-plus",
            Self::DiffMinus => "diff-minus",
            Self::Heading => "heading",
            Self::Quote => "quote",
            Self::Type => "type",
            Self::Variable => "variable",
            Self::Punctuation => "punctuation",
        }
    }
}

pub(super) struct ExpectedRole {
    pub start: usize,
    pub token: &'static str,
    pub role: Role,
}

const fn at(start: usize, token: &'static str, role: Role) -> ExpectedRole {
    ExpectedRole { start, token, role }
}

pub(super) struct Case {
    pub label: &'static str,
    pub code: &'static str,
    pub roles: &'static [ExpectedRole],
}

pub(super) const CASES: &[Case] = &[
    Case {
        label: "asm",
        code: "mov eax, 42\n; a comment\n",
        roles: &[
            at(9, "42", Role::Number),
            at(12, "; a comment", Role::Comment),
        ],
    },
    Case {
        label: "bash",
        code: "name=\"hello\"\necho \"$name\" # note\n",
        roles: &[
            at(5, "\"hello\"", Role::String),
            at(26, "# note", Role::Comment),
        ],
    },
    Case {
        label: "c",
        code: "int answer(void) { return 42; }\n",
        roles: &[at(19, "return", Role::Keyword), at(26, "42", Role::Number)],
    },
    Case {
        label: "c-sharp",
        code: "class Hello { string Say() { return \"hello\"; } }\n",
        roles: &[
            at(0, "class", Role::Keyword),
            at(36, "\"hello\"", Role::String),
        ],
    },
    Case {
        label: "cmake",
        code: "cmake_minimum_required(VERSION 3.20)\nmessage(\"hello\")\n",
        roles: &[
            at(37, "message", Role::Function),
            at(45, "\"hello\"", Role::String),
        ],
    },
    Case {
        label: "dockerfile",
        code: "FROM alpine:3.20\nRUN echo \"hello\"\nCMD [\"echo\", \"hello\"]\n",
        roles: &[
            at(0, "FROM", Role::Keyword),
            at(47, "\"hello\"", Role::String),
        ],
    },
    Case {
        label: "cpp",
        code: "int answer() { return 42; }\n",
        roles: &[at(15, "return", Role::Keyword), at(22, "42", Role::Number)],
    },
    Case {
        label: "css",
        code: "h1 { color: red; margin: 2px; }\n",
        roles: &[at(5, "color", Role::Variable), at(25, "2px", Role::Number)],
    },
    Case {
        label: "dart",
        code: "void main() { print(\"hello\"); }\n",
        roles: &[at(0, "void", Role::Type), at(20, "\"hello\"", Role::String)],
    },
    Case {
        label: "diff",
        code: "--- a/note\n+++ b/note\n@@ -1 +1 @@\n-old\n+new\n",
        roles: &[
            at(34, "-old", Role::DiffMinus),
            at(39, "+new", Role::DiffPlus),
        ],
    },
    Case {
        label: "elixir",
        code: "defmodule Hello do\n  def greet, do: \"hello\"\nend\n",
        roles: &[
            at(0, "defmodule", Role::Keyword),
            at(36, "\"hello\"", Role::String),
        ],
    },
    Case {
        label: "fish",
        code: "set -l name \"hello\"\necho $name\n",
        roles: &[
            at(0, "set", Role::Function),
            at(12, "\"hello\"", Role::String),
        ],
    },
    Case {
        label: "gleam",
        code: "pub fn greet(name: String) { \"hello \" <> name }\n",
        roles: &[
            at(0, "pub", Role::Keyword),
            at(29, "\"hello \"", Role::String),
        ],
    },
    Case {
        label: "go",
        code: "package main\nfunc main() { println(\"hello\") }\n",
        roles: &[
            at(13, "func", Role::Keyword),
            at(35, "\"hello\"", Role::String),
        ],
    },
    Case {
        label: "html",
        code: "<div class=\"hi\">hello</div>\n",
        roles: &[at(1, "div", Role::Keyword), at(11, "\"hi\"", Role::String)],
    },
    Case {
        label: "java",
        code: "class Hello { String say() { return \"hello\"; } }\n",
        roles: &[
            at(0, "class", Role::Keyword),
            at(36, "\"hello\"", Role::String),
        ],
    },
    Case {
        label: "javascript",
        code: "const greet = (name) => \"hello\" + name;\n",
        roles: &[
            at(0, "const", Role::Keyword),
            at(24, "\"hello\"", Role::String),
        ],
    },
    Case {
        label: "json",
        code: "{\"message\": \"hello\", \"count\": 2}\n",
        roles: &[at(12, "\"hello\"", Role::String), at(30, "2", Role::Number)],
    },
    Case {
        label: "julia",
        code: "function greet(name)\n  println(\"hello\")\nend\n",
        roles: &[
            at(0, "function", Role::Keyword),
            at(31, "\"hello\"", Role::String),
        ],
    },
    Case {
        label: "kotlin",
        code: "fun main() { println(\"hello\") }\n",
        roles: &[
            at(0, "fun", Role::Keyword),
            at(21, "\"hello\"", Role::String),
        ],
    },
    Case {
        label: "lua",
        code: "local function greet(name) return \"hello\" end\n",
        roles: &[
            at(0, "local", Role::Keyword),
            at(34, "\"hello\"", Role::String),
        ],
    },
    Case {
        label: "make",
        code: "MESSAGE = hello\nall:\n\t@echo $(MESSAGE)\n",
        roles: &[
            at(16, "all", Role::Function),
            at(30, "MESSAGE", Role::String),
        ],
    },
    Case {
        label: "markdown",
        code: "# Heading\n\n> quoted text\n",
        roles: &[
            at(2, "Heading", Role::Heading),
            at(13, "quoted text", Role::Quote),
        ],
    },
    Case {
        label: "nix",
        code: "let name = \"hello\"; in name\n",
        roles: &[
            at(0, "let", Role::Keyword),
            at(11, "\"hello\"", Role::String),
        ],
    },
    Case {
        label: "ocaml",
        code: "let greet name = print_endline \"hello\"\n",
        roles: &[
            at(0, "let", Role::Keyword),
            at(31, "\"hello\"", Role::String),
        ],
    },
    Case {
        label: "php",
        code: "function greet($name) { return \"hello\"; }\n",
        roles: &[
            at(0, "function", Role::Keyword),
            at(31, "\"hello\"", Role::String),
        ],
    },
    Case {
        label: "python",
        code: "def greet(name):\n    return \"hello\"\n",
        roles: &[
            at(0, "def", Role::Keyword),
            at(28, "\"hello\"", Role::String),
        ],
    },
    Case {
        label: "r",
        code: "greet <- function(name) paste(\"hello\", name)\n",
        roles: &[
            at(9, "function", Role::Keyword),
            at(30, "\"hello\"", Role::String),
        ],
    },
    Case {
        label: "ql",
        code: "import java\nfrom Method m\nselect m\n",
        roles: &[at(0, "import", Role::Keyword), at(17, "Method", Role::Type)],
    },
    Case {
        label: "ruby",
        code: "def greet(name)\n  puts \"hello\"\nend\n",
        roles: &[
            at(0, "def", Role::Keyword),
            at(23, "\"hello\"", Role::String),
        ],
    },
    Case {
        label: "rust",
        code: "fn greet() { println!(\"hello\"); }\n",
        roles: &[
            at(0, "fn", Role::Keyword),
            at(22, "\"hello\"", Role::String),
        ],
    },
    Case {
        label: "scala",
        code: "object Hello { def greet = println(\"hello\") }\n",
        roles: &[
            at(0, "object", Role::Keyword),
            at(35, "\"hello\"", Role::String),
        ],
    },
    Case {
        label: "sql",
        code: "SELECT name FROM posts WHERE id = 42;\n",
        roles: &[at(0, "SELECT", Role::Keyword), at(34, "42", Role::Number)],
    },
    Case {
        label: "swift",
        code: "func greet() { print(\"hello\") }\n",
        roles: &[
            at(0, "func", Role::Keyword),
            at(21, "\"hello\"", Role::String),
        ],
    },
    Case {
        label: "toml",
        code: "title = \"hello\"\ncount = 2\n",
        roles: &[at(8, "\"hello\"", Role::String), at(24, "2", Role::Number)],
    },
    Case {
        label: "typescript",
        code: "const greet = (name: string): string => \"hello\" + name;\n",
        roles: &[
            at(0, "const", Role::Keyword),
            at(40, "\"hello\"", Role::String),
        ],
    },
    Case {
        label: "tsx",
        code: "const Greeting = () => <h1>hello</h1>;\n",
        roles: &[at(0, "const", Role::Keyword), at(34, "h1", Role::Keyword)],
    },
    Case {
        label: "xml",
        code: "<root id=\"hi\"><item>hello</item></root>\n",
        roles: &[at(1, "root", Role::Keyword), at(9, "\"hi\"", Role::String)],
    },
    Case {
        label: "yaml",
        code: "title: hello\ncount: 2\n",
        roles: &[at(0, "title", Role::Variable), at(20, "2", Role::Number)],
    },
    Case {
        label: "zig",
        code: "const std = @import(\"std\");\npub fn main() void { std.debug.print(\"hello\", .{}); }\n",
        roles: &[
            at(0, "const", Role::Keyword),
            at(65, "\"hello\"", Role::String),
        ],
    },
    Case {
        label: "elisp",
        code: super::ELISP,
        roles: &[
            at(0, "(", Role::Punctuation),
            at(1, "progn", Role::Keyword),
            at(9, "add-to-list", Role::FunctionCall),
            at(41, "\"melpa\"", Role::String),
            at(84, "package-list-packages", Role::FunctionCall),
        ],
    },
    Case {
        label: "haskell",
        code: "{-# LANGUAGE OverloadedStrings #-}\nmodule Main where\nmain = putStrLn \"hello\"\n",
        roles: &[
            at(35, "module", Role::Keyword),
            at(69, "\"hello\"", Role::String),
        ],
    },
];
