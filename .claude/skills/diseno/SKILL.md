---
name: diseno
description: Sistema de diseño de la marca, la landing y la web (dirección visual, tokens, tipografía, movimiento, referencias). Úsala al diseñar o maquetar cualquier pantalla o componente visual del proyecto.
---

# Sistema de diseño

> Estado: **borrador**. La dirección visual, el nombre y los colores de marca están por decidir con el autor.
> Rellena las secciones marcadas como PENDIENTE a medida que se decidan; no las inventes.

## Referencia principal
- **entire.io**, sobre todo el hero. Sirve de inspiración, **no se copia**: se toma el nivel de calidad, el ritmo y la sensación, no la maquetación literal, los textos ni los recursos gráficos.
- Antes de diseñar una sección, analiza la referencia (estructura, jerarquía tipográfica, espacio, movimiento) y anota aquí qué se toma y qué se cambia.

## Decisiones de la landing (tomadas)
- Nombre: **Hebra**, comando `hb`. Metáfora: cada persona o agente es una hebra en una tela común.
- Mensaje principal del hero: **hecho para agentes** (control de versiones para cuando la mayoría del código lo escriben agentes).
- Idioma: **inglés**.
- Tema: **claro por defecto**, con modo oscuro opcional.
- Secciones bajo el hero, una por funcionalidad:
  1. Workspaces por agente (`hb start`, aislamiento instantáneo).
  2. Coordinación en tiempo real (presencia y reservas).
  3. Merge que entiende código (por estructura; los conflictos se guardan y no bloquean).
  4. Procedencia y permisos (persona, agente, prompt y tarea; permisos por agente; aprobación humana).

## Análisis de entire.io (2026-10-15)
Lo que se toma: hero centrado con una píldora de anuncio encima del titular; titular de dos líneas, seminegrita y con interlineado ajustado, con una **palabra que rota** (reservando el ancho máximo para que nada salte); párrafo corto y un único CTA principal; fila de iconos y etiquetas al pie del hero que enlaza a las secciones; secciones con una **etiqueta en mono, mayúsculas y color de acento** sobre un título centrado; **líneas guía verticales** que enmarcan el contenido; una maqueta del producto real (CLI o web) en vez de ilustraciones.
Lo que se cambia: entire.io es oscuro y Hebra es **claro** (papel cálido); su visual de fondo es un paisaje y el de Hebra son **hebras de colores** (agentes) que convergen en una línea principal; su tipografía es propia y Hebra usa Geist con una serif itálica para la palabra que rota.

## Dirección visual (decidida por Claude, revisable por el autor)
- Papel cálido de fondo, tinta casi negra y **rojo hilo** (bermellón, `oklch(0.6 0.2 30)`) como acento.
- Colores de hebra para los agentes: rojo, verde azulado, ámbar, violeta y azul.
- Tipografía: Geist (interfaz y titulares), Geist Mono (código y etiquetas) e Instrument Serif en itálica (palabra que rota).
- La terminal se muestra en oscuro también en el tema claro, como contrapunto.
- Movimiento: el dibujo de las hebras al cargar y la rotación de la palabra; nada más. Todo se desactiva con `prefers-reduced-motion`.

## Voz y texto (para que no suene a IA)
El autor pidió expresamente quitar el "tono de IA". Reglas:
- Nada de rayas largas (—). Frases cortas con punto.
- Nada de eslóganes "X, not Y" ("Conflicts are data, not roadblocks"), ni tríadas de adjetivos ("isolated, coordinated, and never blocked"), ni "built for the X era", ni juegos de palabras forzados.
- Concreto antes que grandilocuente: decir qué hace el comando y qué pasa después (`hb start` crea una carpeta con un cambio nuevo).
- Honestidad visible: lo que aún no funciona se dice (lista "Known gaps"). Ninguna cifra sin fuente: las de la franja de datos salen de `scripts/stress.sh`.
- Un poco de humor seco está bien ("Try it on a repo you don't mind breaking").
- Layout: secciones numeradas con cabecera a la izquierda (no la plantilla centrada de etiqueta, título e intro repetida); sin brillos degradados ni retícula de puntos; radios contenidos (`rounded-lg`) y sombras sobrias.

## Principios
1. **Una idea por sección.** La landing explica un concepto cada vez: qué es, por qué ahora, cómo funciona, empieza.
2. **Enseñar el producto real.** Terminal, diffs, agentes trabajando en paralelo: mejor que ilustraciones genéricas.
3. **Contenido técnico creíble.** Ejemplos de comandos y salidas que sean coherentes con la CLI real.
4. **Rendimiento como parte del diseño.** El hero se ve completo sin esperar a JavaScript.
5. **El movimiento tiene propósito** y respeta `prefers-reduced-motion`.

## Tokens (PENDIENTE)
Se definen como variables CSS en `:root` y se exponen a Tailwind. Esperado:
- Color: fondo, superficie, borde, texto (primario, secundario, terciario), acento y estados (éxito, aviso, error).
- Modo claro y oscuro (PENDIENTE: ¿oscuro por defecto?).
- Tipografía: una sans para interfaz y titulares y una mono para código (PENDIENTE).
- Escala de espaciado, radios, sombras y duraciones o curvas de animación.

## Skills globales que conviene usar
- `emil-design-eng` y `apple-design`: pulido de interfaz y detalles.
- `animate`, `review-animations` y `find-animation-opportunities`: movimiento.
- `break-ui`: probar componentes con datos extremos.
- `mobile-native`: comportamiento en móvil.
