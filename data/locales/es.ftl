## CLI
cli-about = Convierte cualquier servidor MCP en un único servidor MCP en modo código.
cli-config = Ruta del archivo de configuración
cli-lang = Idioma de la interfaz (en, es, …)
cli-serve = Sirve el servidor MCP en modo código por stdio
cli-run = Ejecuta una vez un archivo JavaScript contra los servidores configurados
cli-run-file = Archivo JavaScript, o - para stdin
cli-doctor = Conecta con los servidores configurados y muestra las herramientas
cli-history = Muestra las ejecuciones recientes
cli-history-limit = Cuántas ejecuciones mostrar
cli-init = Escribe un archivo de configuración inicial
cli-types = Imprime las declaraciones TypeScript que recibe el modelo

## Messages
doctor-ok = ✓ { $server }: { $count ->
    [one] { $count } herramienta
   *[other] { $count } herramientas
}
doctor-failed = ✗ { $server }: { $reason }
doctor-total = { $servers ->
    [one] { $servers } servidor
   *[other] { $servers } servidores
}, { $tools ->
    [one] { $tools } herramienta
   *[other] { $tools } herramientas
}.
init-written = Configuración escrita en { $path }. Agrega tus servidores en "mcpServers".
init-exists = { $path } ya existe; no se sobrescribió.
history-empty = Todavía no hay ejecuciones registradas.
store-disabled = El historial está desactivado ("store": "none").

## Errors (stable codes, see docs/es/errors.md)
CM0001 = No se pudo leer la configuración: { $reason }
    .hint = Ejecuta `any-mcp-codemode init` para crear una, o usa --config <archivo>.
CM0002 = No hay servidores MCP configurados.
    .hint = Agrega al menos una entrada en "mcpServers" de { $path }.
CM0003 = No se pudo abrir el almacén de historial: { $reason }
CM0004 = No se pudo leer { $path }: { $reason }
CM0005 = La ejecución no terminó ({ $status }): { $reason }
