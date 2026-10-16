---
name: diseno
description: Sistema de diseño de la marca, la landing y la web (dirección visual, tokens, tipografía, movimiento, referencias). Úsala al diseñar o maquetar cualquier pantalla o componente visual del proyecto.
---

# Sistema de diseño

> Estado: **decidido** (2026-10-16). Cambios de dirección, siempre con el autor.

## Referencia histórica
- **entire.io**, sobre todo el hero. Sirve de inspiración, **no se copia**: se toma el nivel de calidad, el ritmo y la sensación, no la maquetación literal, los textos ni los recursos gráficos.
- Antes de diseñar una sección, analiza la referencia (estructura, jerarquía tipográfica, espacio, movimiento) y anota aquí qué se toma y qué se cambia.

## Dirección visual (decidida por el autor, 2026-10-16)
**Herramienta de precisión**, en la línea de Linear y Vercel. Sustituye a la dirección editorial cálida anterior (serif, papel, hilos de colores), que el autor descartó por genérica, confusa y poco pulida.

- **Tema oscuro por defecto**, claro opcional (`data-theme="light"`, guardado en `localStorage`).
- **Neutros grafito** con un tinte frío (hue 260) y **un único color de señal: cian** (`--accent`) para foco, acción principal de la landing y elementos activos. Nada más usa acento.
- **Colores semánticos**: `--ok` (verde), `--warn` (ámbar), `--err` (rojo), `--info` (azul) y `--agent` (violeta). Los agentes son violeta y cuadrados redondeados en avatares; las personas son círculos neutros.
- **Tipografía**: solo Geist y Geist Mono. Nada de serif. Mono para datos (ids, bookmarks, comandos, contadores).
- **Forma**: radios contenidos (`rounded-md` en controles, `rounded-lg` en tarjetas), bordes de 1px, casi sin sombras (solo `shadow-pop` en popovers y paleta).
- **Densidad**: app a 13–14px, filas de 44px, títulos de página a 22px. La landing usa titulares grandes (hasta 76px) y aire.
- **Movimiento**: solo de respuesta (pulsar, drawer, popover, 140–250 ms, `--ease-out`). Las páginas y listas de la app **no** entran animadas; la paleta ⌘K abre sin animación. La landing usa `.rise` al cargar y `.reveal` al hacer scroll (CSS, sin JS).
- **Recetas compartidas** en `apps/web/lib/ui.ts` (`button()`, `control`, `TONE_BADGE`) y componentes en `components/hub/ui.tsx` y `components/hub/forms.tsx`. Usarlos siempre en vez de clases sueltas.

## Landing (estructura actual)
1. Hero centrado: píldora, titular «Version control for people and agents», párrafo que dice qué hace y para quién, dos CTA (Get started y Self-host or Cloud) y una **captura del producto hecha en HTML** (`components/landing/product-shot.tsx`), no una ilustración.
2. Franja «Works with» con los agentes compatibles.
3. How it works: tres pasos con terminales cuya salida coincide con la CLI real.
4. Features: rejilla de 12 con icono, título y una frase; las de Cloud se marcan.
5. For agents, comparación con Git, Get started (comandos copiables y las dos ediciones) y Known gaps.

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
Definidos en `apps/web/app/globals.css` (`:root` oscuro y `:root[data-theme="light"]`) y expuestos a Tailwind con `@theme inline`: `bg`, `surface`, `surface-2`, `surface-3`, `line`, `line-strong`, `ink`, `ink-2`, `ink-3`, `accent`, `ok`, `warn`, `err`, `info`, `agent` y `term-*` (la terminal siempre es oscura).

## Skills globales que conviene usar
- `emil-design-eng` y `apple-design`: pulido de interfaz y detalles.
- `animate`, `review-animations` y `find-animation-opportunities`: movimiento.
- `break-ui`: probar componentes con datos extremos.
- `mobile-native`: comportamiento en móvil.
