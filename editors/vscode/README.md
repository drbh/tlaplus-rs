# TSL for Visual Studio Code

Syntax highlighting for the experimental `.tsl` specification language in
[tlaplus-rs](https://github.com/drbh/tlaplus-rs). Files open in **TSL** language
mode automatically after installing the extension.

Highlights declarations, action and property names, enums, types, state,
constants, guards, updates, choices, temporal operators, fairness, numbers,
Booleans, strings, functions, collection operations, imports, refinement types,
membership checks (`is`), records and fields, views, domains, and composed actions. Also supports `//` comment toggling, bracket matching,
automatic bracket closing, and indentation.

Colors follow your existing theme through standard
[TextMate scopes](https://code.visualstudio.com/api/language-extensions/syntax-highlight-guide).
No theme changes or runtime extension code are required. This is syntax support;
it does not provide type checking, completion, or a language server. Capitalized
references use a constant-style scope by convention; resolving their actual
declarations would require semantic highlighting.

## Install locally

This extension is not published to the Marketplace. Package it from this directory
using the command below, then install the resulting VSIX:

```sh
npx --yes @vscode/vsce@4.0.0 package --skip-license --out tsl-language.vsix
code --install-extension tsl-language.vsix
```

If VS Code was already open, run **Developer: Reload Window** from the Command
Palette. Open a `.tsl` file; the status bar should show **TSL**. If an existing
file association overrides it, use **Change Language Mode → TSL**, or set:

```json
"files.associations": {
  "*.tsl": "tlaplus-tsl"
}
```

To preview changes without installing, from the repository root:

```sh
code --extensionDevelopmentPath="$PWD/editors/vscode" "$PWD"
```

Use **Developer: Inspect Editor Tokens and Scopes** to inspect highlighting.
The grammar matches the current compiler's syntax; unsupported future language
features are not advertised.
