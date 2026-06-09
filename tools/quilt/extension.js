// Quilt VS Code extension: launches the `quilt-lsp` language server and
// connects it as the language client for `.quilt` files. The syntax grammar and
// arrow-glyph keybindings are contributed declaratively in package.json; this
// file only adds the LSP client.

const { workspace, window } = require("vscode");
const { LanguageClient, TransportKind } = require("vscode-languageclient/node");

let client;

function activate(context) {
  const config = workspace.getConfiguration("quilt-lsp");
  const command = config.get("serverPath") || "quilt-lsp";

  // Pass the per-language downstream server overrides through to the server's env.
  const env = Object.assign({}, process.env);
  const overrides = {
    rustAnalyzerPath: "QUILT_LSP_RUST_ANALYZER",
    pythonServerPath: "QUILT_LSP_PYTHON_SERVER",
    wgslServerPath: "QUILT_LSP_WGSL_SERVER",
  };
  for (const [setting, envVar] of Object.entries(overrides)) {
    const val = config.get(setting);
    if (val && val.trim() !== "") {
      env[envVar] = val;
    }
  }
  if (!env.RUST_LOG) {
    env.RUST_LOG = "info";
  }

  const exec = { command, transport: TransportKind.stdio, options: { env } };
  const serverOptions = { run: exec, debug: exec };

  const clientOptions = {
    documentSelector: [{ scheme: "file", language: "quilt" }],
    outputChannel: window.createOutputChannel("Quilt LSP"),
  };

  client = new LanguageClient(
    "quilt-lsp",
    "Quilt LSP",
    serverOptions,
    clientOptions
  );

  client.start().catch((err) => {
    window.showErrorMessage(
      `Quilt LSP failed to start (${command}): ${err.message}. ` +
        `Set "quilt-lsp.serverPath" or add quilt-lsp to your PATH.`
    );
  });

  context.subscriptions.push({ dispose: () => client && client.stop() });
}

function deactivate() {
  return client ? client.stop() : undefined;
}

module.exports = { activate, deactivate };
