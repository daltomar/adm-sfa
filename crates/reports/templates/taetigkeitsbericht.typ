// Tätigkeits- und Geschäftsbericht
// Inputs: see reports/src/statutory.rs :: build_dict
#import sys: inputs

#set document(title: "Tätigkeitsbericht")
#set page(paper: "a4", margin: 2cm)
#set text(size: 10pt, lang: "de")
#show heading: it => {
  v(0.4cm)
  text(size: 11pt, weight: "bold")[#it.body]
  v(0.15cm)
  line(length: 100%, stroke: 0.4pt + luma(180))
  v(0.1cm)
}

// ── Header ───────────────────────────────────────────────────────────────────

#rect(width: 100%, inset: 8pt, stroke: 0.5pt)[
  #text(size: 12pt, weight: "bold")[#inputs.org_name] \
  #text(size: 9pt)[#inputs.org_address] \
  #text(size: 9pt)[Steuernummer: #inputs.org_tax_number] \
  #text(size: 9pt)[Mitglieder zum 31.12.#inputs.year: #inputs.member_count]
]

#v(0.4cm)
#text(size: 14pt, weight: "bold")[Tätigkeits- und Geschäftsbericht]
#linebreak()
#text(size: 10pt)[Geschäftsjahr 01.01.#inputs.year – 31.12.#inputs.year]
#v(0.5cm)
#line(length: 100%, stroke: 0.5pt)

// ── 1. Vereinszweck ──────────────────────────────────────────────────────────

= 1. Vereinszweck

Der Verein fördert den Sport (§ 52 Abs. 2 Nr. 21 AO), insbesondere den
Skateboardsport und die Skateboard-Kultur in Brasilien. Ziel ist es, Kindern
und Jugendlichen in sozial schwächeren Vierteln durch die Bereitstellung von
Skateboard-Material und die Unterstützung von lokalen Projekten Zugang zu
dieser Sportart zu ermöglichen.

// ── 2. Durchgeführte Maßnahmen ───────────────────────────────────────────────

= 2. Durchgeführte Maßnahmen

#if inputs.outbound_events.len() > 0 [
  Im Geschäftsjahr #inputs.year wurden folgende Spendenmaßnahmen durchgeführt:

  #table(
    columns: (2.5cm, 1fr, 2cm),
    stroke: (x, y) => (bottom: 0.4pt + luma(200)),
    fill: (_, y) => if y == 0 { luma(230) } else if calc.even(y) { luma(248) } else { none },
    inset: (x: 5pt, y: 4pt),
    text(weight: "bold")[Datum],
    text(weight: "bold")[Empfänger / Projekt],
    align(right, text(weight: "bold")[Artikel]),
    ..inputs.outbound_events.map(e => (
      e.date,
      e.recipient,
      align(right)[#e.item_count],
    )).flatten(),
  )

  #v(0.3cm)
] else [
  Im Geschäftsjahr #inputs.year wurden keine Spendenmaßnahmen im System erfasst.
  #v(0.3cm)
]

#for para in inputs.tb_activities_paras [
  #para
  #v(0.2cm)
]

// ── 3. Sonstige Tätigkeiten ──────────────────────────────────────────────────

= 3. Sonstige Tätigkeiten

#for para in inputs.tb_continuous_paras [
  #para
  #v(0.2cm)
]

// ── 4. Ausblick ───────────────────────────────────────────────────────────────

#let next_year = str(int(inputs.year) + 1)
= 4. Ausblick #next_year

#for para in inputs.tb_outlook_paras [
  #para
  #v(0.2cm)
]

// ── 5. Vorstand und Mitgliederversammlung ─────────────────────────────────────

= 5. Vorstand und Mitgliederversammlung

Im Berichtsjahr #inputs.year fand eine gemeinsame Vorstandssitzung und
Mitgliederversammlung statt.

// ── Footer ────────────────────────────────────────────────────────────────────

#v(1cm)
#text(size: 9pt, fill: luma(100))[Erstellt am: #inputs.generated]

#v(1.5cm)
#grid(columns: (1fr, 1fr), gutter: 1cm,
  [
    #line(length: 100%, stroke: 0.5pt) \
    #text(size: 9pt)[Ort, Datum / Unterschrift Vorstand]
  ],
  [
    #line(length: 100%, stroke: 0.5pt) \
    #text(size: 9pt)[Unterschrift Kassenwart]
  ]
)
