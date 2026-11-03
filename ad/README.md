# Kast — «Un solo lugar» (anuncio de 21,2 s)

Anuncio conceptual de Kast hecho entero con un pipeline programático reproducible: Blender/Cycles automatizado con
Python para el mundo 3D, NumPy para la música y el diseño sonoro, ElevenLabs para la locución y ffmpeg para el máster.
El brief está en el documento «Brief del anuncio de Kast».

## Entregables

| Archivo | Contenido |
| --- | --- |
| `out/Kast_UnSoloLugar_1920x1080.mp4` | Máster horizontal: 1920 × 1080, 60 fps, 21,2 s, H.264 High + AAC 320 kbps |
| `out/Kast_UnSoloLugar_1080x1920.mp4` | Máster vertical: 1080 × 1920, misma duración, recompuesto (otra cámara del mismo mundo) |
| `out/qa/informe.md` y `out/qa/h`, `out/qa/v` | Validación técnica y capturas de los momentos clave |
| `audio/out/vo.wav`, `music.wav`, `sfx.wav`, `mix.wav` | Locución colocada, música original, efectos y mezcla final (48 kHz, 24 bits) |
| `audio/takes3/eduardo.mp3` | Toma de la locución con Eleven v4, con sus marcas de tiempo por carácter (`eduardo.json`). Las tomas descartadas (`brian`, `nyko`, `diego` y las femeninas de `takes/` y `takes2/`) se quedan en local |

`out/` y `audio/out/` no se versionan (renders y audio generado); se regeneran con los pasos de abajo.

## Rehacerlo

```bash
python ad/render/textures.py                      # texturas de las pantallas
python ad/audio/vo.py                             # locución en sus anclas (necesita la toma)
python ad/audio/music.py && python ad/audio/sfx.py
python ad/audio/mix.py                            # -14 LUFS, pico real <= -1,25 dBTP
blender -b --factory-startup -P ad/render/build.py -- --aspect h --samples 40 --range 0-1271 --out ad/out/h/f_
blender -b --factory-startup -P ad/render/build.py -- --aspect v --samples 40 --range 0-1271 --out ad/out/v/f_
python ad/tools/encode.py h && python ad/tools/encode.py v
python ad/tools/qa.py
```

Para generar locuciones nuevas: `python ad/audio/eleven.py voices` y
`python ad/audio/eleven.py say <voz> <salida.mp3> [semilla] [estabilidad] [modelo]` (modelo por defecto `eleven_v4`;
la clave se lee de la variable de entorno `ELEVENLABS_API_KEY` y nunca se imprime). Para cambiar de toma, se apunta
`VO_TAKE` en `timeline.py` y se ajustan los cortes de `VO` a los silencios reales de la nueva toma.

## Estructura

```
timeline.py          línea de tiempo maestra (voz, escenas, golpes, rejilla de 100 BPM): la comparten imagen y sonido
render/kit.py        materiales físicos, geometría con biseles, texto, curvas de animación (muelles, anticipación…)
render/assets.py     el cursor (protagonista, con claves de forma), el logo (de assets/kast.svg), monitores y dispositivos
render/world.py      render (Cycles), ambiente, composición (bruma por profundidad, bloom) y la cámara única
render/stations*.py  las diez escenas
render/build.py      monta el mundo, hornea la animación fotograma a fotograma y renderiza
audio/               locución (ElevenLabs), música, efectos (síntesis con NumPy) y mezcla
tools/               máster (encode), validación (qa), hojas de contacto y revisión de secuencias
```

## Registro de dependencias

| Herramienta | Versión | Uso |
| --- | --- | --- |
| Blender | 5.2.2 LTS (Cycles, OptiX en una GeForce RTX 5050) | Mundo 3D, animación, render, composición |
| Python | 3.14.3 | Orquestación, texturas, audio |
| NumPy / Pillow | 2.4.2 / 12.2.0 | Síntesis de audio, texturas, revisiones |
| ffmpeg | 8.0.1 | Decodificación, sonoridad (EBU R128), máster |
| ElevenLabs | modelo `eleven_v4`, voz «Eduardo» (tBkfgTXKnmHoYewBvh7e, masculina, mexicana, cálida), semilla 21, estabilidad 0,5 | Locución |
| Tipografía | Segoe UI (la de la interfaz de Kast, instalada en Windows) | Textos. Para difundir el anuncio fuera de un uso interno, cambiar a Inter Display (OFL) |

## Decisiones de producción respecto del brief

- **Voz masculina y cercana**: Eduardo con Eleven v4. La lista de plataformas se corta en «Windows» / «Mac, iPhone…» /
  «hasta Android» y «Un solo lugar» va aparte, para que cada palabra caiga con su imagen. Los cortes se midieron en la
  señal (las marcas de tiempo de la API llegan hasta 0,19 s tarde). La voz pasa por un compresor 3:1 y se deja a
  -16 dBFS RMS de nivel hablado, así el limitador del máster apenas trabaja.
- **Locución más corta.** El guion completo no cabía en 19,2 s sin acelerar la voz. Se unió «Tu otro equipo, a un
  clic… Y, de repente, está en tu monitor» en «Y, de repente, tu otro equipo… está en tu monitor» y se quitó «Un
  teléfono». Cada frase se coloca en su ancla moviendo sólo las pausas.
- **Tiempos de las escenas** (ver `timeline.py`): el aterrizaje final cae en 17,20 s y «Kast» en 17,55 s; la firma
  sonora, en 18,40 s. Todo cae en la rejilla de 100 BPM.
- **Calibración** con una barra general «Brillo · todas» que mueve las tres de cada monitor a su valor calibrado (en vez
  de cinco puntos de una curva): se entiende mejor en 2 s.
- **Cuatro plataformas** como dioramas en las esquinas de un gran icono de Kast trazado en el suelo; al final, desde
  arriba, se lee el logo con el mensaje dentro.
- **Etalonaje**: el máster levanta el negro del render al grafito del brief (#0A0A0C) con un pie suave; los planos más
  oscuros (la estación de Windows antes de encenderse, la bajada al aterrizaje) se leen como espacio, no como negro.
- **Cámara más tranquila** (para que no maree): la trayectoria se suaviza en el tiempo con una gaussiana de 0,16 s
  (`CameraRig.COMFORT`), sin mover claves ni sincronías, y los dos giros más bruscos tienen más tiempo (la subida a la
  vista cenital llega a 14,80 s y la bajada al aterrizaje empieza a 16,30 s). El barrido máximo pasó de 5,4 a 2,3
  anchos de cuadro por segundo y la velocidad máxima de 30 a 18 m/s. Con la cámara suavizada, ningún giro pasa por
  un plano vacío ni atraviesa nada: las pantallas del vuelo de la escena 2 quedan fuera de su pasillo, la línea de la
  confirmación vuela baja sin tocar el suelo (a ras de él salía del cuadro) y la estación de Windows se enciende
  mientras la cámara va hacia ella.
- **La pantalla de Android sale del suelo** al llegar la cámara (13,25 s): de pie desde el principio tapaba la estación
  de Windows justo cuando la voz la nombra.
- **Aterrizaje anticipado**: la cámara encuadra el punto de aterrizaje antes de que entre el cursor, y su brillo se
  proyecta en el suelo mientras cae (un charco lavanda que se cierra al acercarse).
- **Versión vertical**: misma trayectoria, con focal más cerrada y la cámara desplazada en cada escena hacia lo
  importante; la firma se reparte en tres líneas.
