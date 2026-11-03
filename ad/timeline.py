"""Línea de tiempo maestra del anuncio (segundos). La comparten el render 3D y el audio.

Se fijó a partir de la rejilla musical (100 BPM, un tiempo = 0,6 s, con el primer tiempo en el clic inicial, 0,40 s)
y de la locución: la toma de Eduardo con Eleven v4. Las frases de la voz se colocan en sus anclas moviendo sólo las
pausas (nunca acelerándola).
"""
FPS = 60
AD = 19.2          # anuncio
TOTAL = 21.2       # con la pantalla final
FRAMES = round(TOTAL * FPS)   # 1272

BEAT = 0.6
GRID0 = 0.40       # primer tiempo (el clic)


def beat(n):
    """Tiempo n de la rejilla (puede ser fraccionario: 0,5 = corchea)."""
    return GRID0 + n * BEAT


# Locución: (inicio en la toma, fin en la toma, colocación en el anuncio) de cada frase. Los cortes son los silencios
# reales de la toma (medidos en la señal: las marcas de tiempo de ElevenLabs llegan hasta 0,19 s tarde). La lista de
# plataformas y «Un solo lugar» se separan para que cada palabra caiga con su imagen.
VO_TAKE = "ad/audio/takes3/eduardo.mp3"
VO = [
    (0.03, 2.10, 0.52),     # Tres monitores. Dos equipos.
    (2.355, 4.16, 2.90),    # Un brillo que por fin coincide.            «coincide» 4,11
    (4.425, 6.31, 4.90),    # Una disposición para cada momento.
    (6.60, 10.055, 7.31),   # Y, de repente, tu otro equipo… está en tu monitor.   «tu otro equipo» 8,35, «está» 9,71
    (10.36, 11.06, 11.16),  # Windows,
    (11.065, 12.27, 12.06),  # Mac, iPhone…                              «iPhone» 12,70
    (12.455, 13.18, 13.60),  # hasta Android.                           «Android» 13,84
    (13.555, 14.65, 14.58),  # Todas tus pantallas.
    (14.825, 15.66, 15.98),  # Un solo lugar.
    (16.035, 16.525, 17.55),  # Kast.
]

# Escenas (inicio, fin)
S1 = (0.00, 1.55)    # El clic
S2 = (1.55, 2.90)    # El universo de tus pantallas
S3 = (2.90, 4.85)    # Tus monitores, iguales
S4 = (4.85, 7.25)    # Una disposición para cada momento
S5 = (7.25, 9.45)    # El otro equipo
S6 = (9.45, 11.00)   # La otra pantalla
S7 = (11.00, 14.55)  # Cuatro mundos, un lugar
S8 = (14.55, 17.00)  # La gran idea
S9 = (17.00, 19.20)  # El cierre
S10 = (19.20, 21.20)  # Firma

# Momentos clave
CLICK = 0.40                 # el cursor golpea la cámara
LOGO_DRAW = (0.95, 1.40)
PRESS = 2.50                 # la línea pulsa el icono de Kast
MATCH = 4.20                 # los tres monitores llegan al mismo blanco («coincide»)
CONFIRM1 = beat(7)           # 4,60: confirmación
IGNITION = beat(8)           # 5,20: encendido de las disposiciones
CONNECT_CLICK = 8.35         # el cursor pulsa Conectar
CONFIRM2 = beat(15)          # 9,40: conectado
MAC_APPEARS = 9.70           # «está en tu monitor»
PLATFORMS = {"windows": beat(18), "mac": beat(19.5), "iphone": beat(20.5), "android": beat(22.5)}  # 11,2 12,1 12,7 13,9
TEXT1 = beat(23.5)           # 14,50: TODAS TUS PANTALLAS.
TEXT2 = beat(26)             # 16,00: UN SOLO LUGAR.
PAUSE = beat(27)             # 16,60: la música se retira
LANDING = beat(28)           # 17,20: el cursor aterriza
LOGO2 = (17.30, 17.75)
KAST_VO = 17.55
FANFARE = beat(30)           # 18,40
