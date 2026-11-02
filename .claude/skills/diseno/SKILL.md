---
name: diseno
description: Sistema de diseño de la marca, la landing y la web (dirección visual, tokens, tipografía, movimiento, referencias). Úsala al diseñar o maquetar cualquier pantalla o componente visual del proyecto.
---

# Sistema de diseño

> Estado: **decidido** (2026-11-02). Cambios de dirección, siempre con el autor.

## Historia
- 2026-10-16: «herramienta de precisión» (Linear/Vercel). Descartada.
- 2026-10-17: landing pixel. Gustó la landing; el hub no.
- 2026-10-19: dirección «Console» (panel cálido, teclas, etiquetas impresas). Sustituida.
- 2026-11-02: el autor pidió adaptar el diseño al kit [Bencho](https://bencho.dev). Es la dirección actual.

## Dirección visual: Bencho con la identidad de Hebra
- **Neutros y una sola acción llena**: tinta sobre blanco, blanco sobre negro. El tema sigue al sistema; `data-theme` lo fija y `hebra-theme` en `localStorage` lo recuerda.
- **Píldoras** para todo lo que se pulsa (`button()` en `lib/ui.ts`, altura 36 px, peso 500). **Tableros** grandes y suaves para todo lo que se lee (`rounded-2xl bg-surface`), sin líneas alrededor. Las líneas solo separan filas dentro de un tablero.
- **Tipografía**: Inter (títulos de peso 500, tracking mínimo) y JetBrains Mono solo para código, comandos e ids. Nada de mayúsculas impresas.
- **`bg-wash`** es el fondo de lo secundario: es translúcido y funciona sobre cualquier superficie.
- **Color solo con significado**: cada hebra tiene su hilo (`--thread-1..4`), las personas son neutras, el rosa (`--err`) es para conflictos y fallos, `--led` para lo que está en marcha.
- **Se queda de Hebra**: el logo tejido, los glifos pixel de agentes y personas, los robots del escenario (`components/landing/cast.tsx`) y la palabra pixel del pie.
- **Sin lo de Console**: teclas, etiquetas numeradas (`01 CHANNELS`), matriz de puntos y marco con líneas.

## Bloques de Bencho
- En `components/bencho/`, adaptados y con su licencia MIT (ver su `README.md`). Solo los gratuitos: los Pro son de pago y su nombre y la cabra no se usan.
- Sin dependencias nuevas: `motion/react` e iconos SVG en línea.
- Un bloque entra si muestra un dato real de Hebra. Hoy: `StackedBars` (actividad del repo), `WorkingIsland` (quién trabaja), `ToastStack` y `OpScrubber` (portada).

## Portada
- Héroe centrado, escenario pixel en un tablero y debajo una pared de tableros, cada uno con una pieza viva del producto y su explicación.
- Lo que aún no funciona se dice («Not there yet»). Las cifras llevan fecha y script.

## Portada y dashboard
- `/` es la portada solo en la web pública (`HEBRA_PUBLIC_SITE=1`) y en `next dev`. Un servidor autoalojado abre el dashboard (`proxy.ts`).
- `/hub` es la portada del dashboard: qué pasa en tus repositorios, y los primeros pasos si no hay ninguno.
- Hebra Cloud no aparece salvo con `NEXT_PUBLIC_HEBRA_CLOUD=1`. El dashboard sigue a `caps.features`, nunca a ese interruptor.

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
Definidos en `apps/web/app/globals.css` (`:root` claro y `:root[data-theme="dark"]`) y expuestos a Tailwind con `@theme inline`: `bg`, `surface`, `surface-2`, `surface-3`, `wash`, `wash-strong`, `line`, `line-strong`, `ink`, `ink-2`, `ink-3`, `accent`, `ok`, `warn`, `err`, `info`, `agent`, `signal` (el LED), `person`, `thread-1..5` y `term-*` (la terminal siempre es oscura). Los nombres que leen los bloques de Bencho (`--fill-slab`, `--fill-on`...) están en el mismo sitio.

## Skills globales que conviene usar
- `emil-design-eng` y `apple-design`: pulido de interfaz y detalles.
- `animate`, `review-animations` y `find-animation-opportunities`: movimiento.
- `break-ui`: probar componentes con datos extremos.
- `mobile-native`: comportamiento en móvil.
