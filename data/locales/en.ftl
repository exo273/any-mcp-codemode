## CLI
cli-about = Turn any MCP servers into one code-mode MCP server.
cli-config = Path to the config file
cli-lang = Interface language (en, es, …)
cli-serve = Serve the code-mode MCP server over stdio
cli-run = Run a JavaScript file once against the configured servers
cli-run-file = JavaScript file, or - for stdin
cli-doctor = Connect to the configured servers and show the tools
cli-history = Show recent executions
cli-history-limit = How many executions to show
cli-init = Write a starter config file
cli-types = Print the TypeScript declarations the model receives

## Messages
doctor-ok = ✓ { $server }: { $count ->
    [one] { $count } tool
   *[other] { $count } tools
}
doctor-failed = ✗ { $server }: { $reason }
doctor-total = { $servers ->
    [one] { $servers } server
   *[other] { $servers } servers
}, { $tools ->
    [one] { $tools } tool
   *[other] { $tools } tools
}.
init-written = Config written to { $path }. Add your servers under "mcpServers".
init-exists = { $path } already exists; not overwritten.
history-empty = No executions recorded yet.
store-disabled = History is disabled ("store": "none").

## Errors (stable codes, see docs/en/errors.md)
CM0001 = Could not read the config: { $reason }
    .hint = Run `any-mcp-codemode init` to create one, or pass --config <file>.
CM0002 = No MCP servers are configured.
    .hint = Add at least one entry under "mcpServers" in { $path }.
CM0003 = Could not open the history store: { $reason }
CM0004 = Could not read { $path }: { $reason }
CM0005 = The execution did not complete ({ $status }): { $reason }
