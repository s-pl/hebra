---
name: diseno
description: Sistema de diseño de la marca, la landing y la web (dirección visual, tokens, tipografía, movimiento, referencias). Úsala al diseñar o maquetar cualquier pantalla o componente visual del proyecto.
---

# Sistema de diseño

> Estado: **decidido** (2026-10-19). Cambios de dirección, siempre con el autor.

## Historia
- 2026-10-16: «herramienta de precisión» (Linear/Vercel). Descartada.
- 2026-10-17: landing pixel (lienzo «Hebra redesign», artboard `Pixel.dc.html`). Gustó la landing; el hub no.
- 2026-10-19: el autor eligió la dirección **B «Console»** para toda la web y pidió combinar la landing pixel con ella.

## Dirección visual: un instrumento (2026-10-19)
- **Tema claro por defecto** (panel cálido `#e6e5df`, tinta `#141414`); oscuro con `data-theme="dark"` (guardado en `localStorage` como `hebra-theme`).
- **Teclas**: los botones son teclas (`.key`, `.key-ink`, `.key-accent`, `.key-danger`) con un borde inferior que se hunde al pulsar. Los atajos van en una tecla pequeña `.cap`.
- **Etiquetas impresas**: `.lbl` (mono, 11px, mayúsculas, tracking). Los módulos se numeran: `01 CHANNELS`.
- **Cada hebra tiene su color** (`--thread-1..4`); las personas son neutras (`--person`); el rosa (`--err`) es solo para conflictos y fallos. `--signal` (azafrán) es la única luz: LEDs, cursor, prompt `hb ▸`.
- **Glifos pixel 5×5** para agentes (cara de robot) y personas (`components/hub/glyph.tsx`). El logo es un tejido 4×4 (`components/logo.tsx`).
- **Tipografía**: Hanken Grotesk y JetBrains Mono (sin ligaduras en código).
- **Piezas de consola** en `components/hub/console.tsx`: `Module`, `DotNumber` (matriz de puntos), `Readout`, `Channel` (tira de canal por hebra).
- **Hub**: sin barra lateral. Barra superior con migas en mono y una fila de pestañas-tecla con atajo de una letra (O, F, V, T, K, Q, L, S). Repositorios y bookmarks en menús y en ⌘K.

## Landing (pixel + consola)
- Marco con líneas: todas las secciones dentro de una columna de 1376px con hairlines a los lados; cabecera y pie en celdas.
- Hero: titular y el **escenario** (`components/landing/cast.tsx`): robots y personas pixel sobre parches tejidos y la banda con la lanzadera. Al pulsar un agente reclama `src/auth.rs`; si lo pulsa otro, los dos se avisan en rosa. Sin terminal en el hero (el autor la quitó).
- La captura del hub está hecha con las piezas reales de la consola (`console-shot.tsx`).
- How it works: navegación lateral y filas con terminales sobre baldosas pixel del color de cada tema.
- Built for agents: celdas de features, cada una con su sticker (`public/stickers/*.png`, hechos en ChatGPT). Sin pared de stickers (el autor la quitó).
- Pie: palabra «hebra» en píxeles, un robot que camina y la banda tejida.
- Movimiento pixel en `globals.css` (`.px-*`): `steps()` para fotogramas, transform/opacity/clip-path, todo apagado con `prefers-reduced-motion`.

## Voz y texto (para que no suene a IA)
El autor pidió expresamente quitar el "tono de IA". Reglas:
- Nada de rayas largas (—). Frases cortas con punto.
- Nada de eslóganes "X, not Y" ("Conflicts are data, not roadblocks"), ni tríadas de adjetivos ("isolated, coordinated, and never blocked"), ni "built for the X era", ni juegos de palabras forzados.
- Concreto antes que grandilocuente: decir qué hace el comando y qué pasa después (`hb start` crea una carpeta con un cambio nuevo).
- Honestidad visible: lo que aún no funciona se dice (lista "Known gaps"). Ninguna cifra sin fuente: las de la franja de datos salen de `scripts/stress.sh`.
- Un poco de humor seco está bien ("Try it on a repo you don't mind breaking").

## Principios
1. **Una idea por sección.** La landing explica un concepto cada vez: qué es, por qué ahora, cómo funciona, empieza.
2. **Enseñar el producto real.** Terminal, diffs, agentes trabajando en paralelo: mejor que ilustraciones genéricas.
3. **Contenido técnico creíble.** Ejemplos de comandos y salidas que sean coherentes con la CLI real.
4. **Rendimiento como parte del diseño.** El hero se ve completo sin esperar a JavaScript.
5. **El movimiento tiene propósito** y respeta `prefers-reduced-motion`.

## Tokens
Definidos en `apps/web/app/globals.css` (`:root` claro y `:root[data-theme="dark"]`) y expuestos a Tailwind con `@theme inline`: `bg`, `surface`, `surface-2`, `surface-3`, `line`, `line-strong`, `ink`, `ink-2`, `ink-3`, `accent`, `ok`, `warn`, `err`, `info`, `agent`, `signal`, `person`, `thread-1..5` y `term-*` (la terminal siempre es oscura).

## Skills globales que conviene usar
- `emil-design-eng` y `apple-design`: pulido de interfaz y detalles.
- `animate`, `review-animations` y `find-animation-opportunities`: movimiento.
- `break-ui`: probar componentes con datos extremos.
- `mobile-native`: comportamiento en móvil.
