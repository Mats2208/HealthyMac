# HealthyMac

Limpiador y monitor para macOS, escrito en Rust, con funciones de primera clase para developers y
para quien ejecuta IA en local. La interfaz está hecha con [GPUI](https://www.gpui.rs).

## Qué lo hace distinto

**Dice cuánto vas a recuperar de verdad.** pnpm, uv y bun construyen sus carpetas con clones APFS de
su almacén global, así que un `.venv` de 961 MB puede liberar solo 45 MB. HealthyMac le pregunta a
APFS qué bloques son exclusivos y separa lo que se libera de lo que seguiría en disco.

**Entiende tus modelos de IA.** Un único inventario para LM Studio, Ollama, Hugging Face, oMLX,
Whisper, llama.cpp, Jan, GPT4All, ComfyUI y otros, con metadatos GGUF, encaje en tu memoria unificada
según el límite real de Metal, modelos incompletos y estado de los runtimes.

**Deduplica sin perder nada.** Si dos runtimes guardan los mismos pesos, los convierte en clones APFS:
cada programa sigue encontrando su archivo y solo se liberan los bloques repetidos.

**No toca lo que no debe.** Una política de seguridad revalida cada ruta justo antes de actuar y
protege credenciales, datos personales, carpetas sincronizadas y almacenes de modelos enteros, aunque
un escáner tenga un bug. Por defecto todo va a la Papelera, y lo que no se regenera nunca va
preseleccionado.

## Funciones

| Página | Qué hace |
|---|---|
| Resumen | Puntuación de salud explicada, uso de disco y escaneo inteligente |
| Sistema | Cachés, registros, Papelera, instantáneas de Time Machine |
| Developer | Xcode, artefactos de proyectos, cachés de paquetes, herramientas, toolchains, Docker |
| IA local | Duplicados, descargas abandonadas, huérfanos, modelos sin usar o que no caben |
| Archivos | Archivos grandes, duplicados, instaladores viejos, copias de iOS |
| Modelos | Inventario, presupuesto de memoria y runtimes en marcha |
| Proyectos | Proyectos con su actividad, artefactos, estado de git y resumen de Docker |
| Puertos | Qué proceso escucha en cada puerto, con su proyecto, y opción de terminarlo |
| Apps | Tamaño, restos, arquitectura y desinstalación |
| Arranque | Agentes y daemons, marcando los huérfanos |
| Mapa de disco | Treemap navegable |
| Monitor | CPU, memoria, red, temperatura, batería y procesos |

## Compilar y ejecutar

Hace falta Rust estable y Xcode. El Metal Toolchain no es necesario: los shaders se compilan al
arrancar.

```bash
cargo build --release -p hm-app
```

```bash
scripts/bundle.sh release
```

```bash
open target/release/HealthyMac.app
```

### Modo seguro para probar

| Variable | Efecto |
|---|---|
| `HEALTHYMAC_DRY_RUN=1` | Nada se borra. La app muestra una insignia de SIMULACIÓN |
| `HEALTHYMAC_HOME=<ruta>` | Trabaja sobre una home de prueba |
| `HEALTHYMAC_DEMO=1` | Datos de ejemplo. Nunca llega al limpiador real |

```bash
HEALTHYMAC_DRY_RUN=1 target/release/HealthyMac.app/Contents/MacOS/HealthyMac
```

## Línea de comandos

```bash
cargo install --path crates/hm-cli
```

```bash
healthymac scan
```

```bash
healthymac scan --area dev --json
```

```bash
healthymac clean <id>
```

`clean` solo simula salvo que añadas `--yes`. También están `models`, `runtimes`, `ports`, `projects`,
`docker`, `apps`, `startup` y `monitor`, con salida JSON.

## Permisos

Sin Acceso total al disco funciona casi todo, y omite lo que macOS protege: Documentos, Escritorio,
Descargas, Música, las bibliotecas de fotos y vídeo, la Papelera, datos de otras apps, las copias de
iOS y la mayor parte de `~/Library`. Al concederlo en Ajustes del Sistema se analizan también esas
carpetas, y se puede vaciar la Papelera desde la app.

## Limitaciones conocidas

- La limpieza real solo se ha ejercitado sobre una home de prueba. Nunca se han ejecutado de verdad
  una desinstalación, `docker * prune`, `brew cleanup`, `ollama rm` ni `xcrun simctl delete`.
- El modo con Acceso total al disco no se ha probado en un Mac real, solo con fixtures.
- Ollama, Jan, GPT4All, ComfyUI, Automatic1111 y Draw Things se han probado solo con fixtures.
- macOS 27 en un M5 Pro es el único sistema probado.
- La firma es ad-hoc: para distribuirla hace falta un Developer ID y notarizarla.
- Solo hay tema oscuro y textos en español.

## Desarrollo

```bash
cargo test --workspace
```

```bash
cargo clippy --workspace --all-targets
```

Las pruebas contra la Papelera real están ignoradas por defecto porque dejan archivos diminutos en
ella:

```bash
cargo test -p hm-core --test real_trash -- --ignored
```

Las reglas y la estructura del proyecto están en [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

Al firmar en ad-hoc, cada compilación es una identidad nueva para macOS, así que los permisos se
vuelven a pedir tras recompilar. Para detectar diálogos de permisos hay que lanzar la app con `open`:
ejecutada desde una terminal, macOS atribuye el acceso a la terminal y el problema no se ve.
