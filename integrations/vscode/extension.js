// Minimal VS Code client for `kor lsp` (spec §12.3). Install: `npm install` here, then
// `code --extensionDevelopmentPath=.` or package it with `vsce package`.
const vscode = require('vscode');
const { LanguageClient } = require('vscode-languageclient/node');

let client;

exports.activate = async (context) => {
  const command = vscode.workspace.getConfiguration('kor').get('path', 'kor');
  client = new LanguageClient(
    'kor',
    'Kor',
    { command, args: ['lsp'] },
    { documentSelector: [{ scheme: 'file' }] },
  );
  context.subscriptions.push(
    vscode.commands.registerCommand('kor.runTaskPrompt', async () => {
      const task = await vscode.window.showInputBox({ prompt: 'Task for Kor' });
      if (task) {
        await client.sendRequest('workspace/executeCommand', {
          command: 'kor.runTask',
          arguments: [task],
        });
      }
    }),
  );
  await client.start();
};

exports.deactivate = () => (client ? client.stop() : undefined);
