---
name: web
description: Convenciones de la web (landing y plataforma) en apps/web con Next.js App Router, TypeScript estricto, Tailwind CSS y pnpm. Úsala al crear o modificar cualquier cosa en apps/web.
---

# Convenciones de la web

## Base
- Next.js con App Router y Server Components por defecto; `"use client"` solo donde haga falta interactividad.
- TypeScript con `strict: true` y `noUncheckedIndexedAccess: true`. Prohibido `any` salvo justificación.
- Tailwind CSS con los tokens del sistema de diseño (skill `diseno`). Nada de colores ni tamaños sueltos en los componentes.
- pnpm como gestor de paquetes. Versiones según la skill `ultimas-versiones`.
- Lint y formato: Biome (una sola herramienta, rápida). Typecheck con `tsc --noEmit`.

## Landing
- Generada estáticamente (sin datos en tiempo de petición) para que cargue al instante.
- Objetivos de rendimiento: LCP < 1,5 s, CLS < 0,05 y JavaScript mínimo en la página inicial.
- Fuentes con `next/font` (auto-hospedadas, sin saltos de diseño).
- Imágenes con `next/image`. SVG en línea para iconos y diagramas.
- Accesible: contraste AA, navegable con teclado, `prefers-reduced-motion` respetado.
- Metadatos completos: título, descripción, Open Graph e imagen social.

## Piezas existentes
- `lib/ui.ts`: recetas de botón y campo. `components/hub/ui.tsx` y `console.tsx`: tarjetas, listas, contadores.
- `components/bencho/`: bloques de Bencho adaptados (ver su `README.md`).
- `components/landing/typed-term.tsx`: terminal que se escribe sola al entrar en pantalla. Su contenido debe reproducir la salida real de `hb`.
- `components/agent-logo.tsx`: logos de agentes desde `simple-icons` (CC0). OpenAI no está en Simple Icons: Codex usa una insignia neutra; no redibujar logos de marcas a mano.
- `lib/docs.ts` y `app/docs`: la documentación de `docs/*.md` servida en la web con `lib/markdown.tsx`.
- Todas las animaciones se desactivan con `prefers-reduced-motion`.

## Antes de dar algo por terminado
`pnpm lint`, `pnpm typecheck` y `pnpm build` sin errores, y revisar la página en móvil (375 px) y escritorio.
