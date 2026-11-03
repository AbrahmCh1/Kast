"""Frases de la locución: dónde empieza y termina cada una en una toma (marcas de tiempo por carácter de
ElevenLabs) y si cabe en su ventana del guion. La voz no se acelera: sólo se mueven las pausas entre frases."""
import json, sys

# (texto con el que empieza la frase, ancla de inicio en el anuncio, siguiente ancla)
PHRASES = [
    ("Tres monitores", 0.60), ("Un brillo", 2.95), ("Una disposición", 4.80), ("Y, de repente", 7.40),
    ("Windows", 11.20), ("Todas tus pantallas", 14.80), ("Kast.", 17.33),
]


def spans(alignment):
    chars = alignment["characters"]
    st, en = alignment["character_start_times_seconds"], alignment["character_end_times_seconds"]
    text = "".join(chars)
    starts = []
    pos = 0
    for p, _ in PHRASES:
        i = text.index(p, pos)
        starts.append(i)
        pos = i + len(p)
    out = []
    for k, i in enumerate(starts):
        j = (starts[k + 1] if k + 1 < len(starts) else len(text)) - 1
        while j > i and not text[j].strip():
            j -= 1
        # Último carácter con sonido (los signos de puntuación duran 0).
        out.append((st[i], en[j], text[i:j + 1]))
    return out


if __name__ == "__main__":
    for path in sys.argv[1:]:
        al = json.load(open(path, encoding="utf-8"))
        print(path)
        sp = spans(al)
        for k, ((a, b, t), (_, anchor)) in enumerate(zip(sp, PHRASES)):
            nxt = PHRASES[k + 1][1] if k + 1 < len(PHRASES) else 19.2
            dur = b - a
            room = nxt - anchor
            print(f"  {anchor:5.2f}  {dur:4.2f} s de {room:4.2f}  {'OK ' if dur <= room - 0.15 else 'NO '} {t}")
