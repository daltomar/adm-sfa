// Aufstellung über das Vereinsvermögen
// Inputs: see reports/src/statutory.rs :: build_dict
#import sys: inputs

#set document(title: "Vermögensaufstellung")
#set page(paper: "a4", margin: 2cm)
#set text(size: 10pt, lang: "de")

#let zero = "0,00"

// ── Header ───────────────────────────────────────────────────────────────────

#text(size: 13pt, weight: "bold")[#inputs.org_name]
#linebreak()
#text(size: 9pt)[#inputs.org_address]
#linebreak()
#text(size: 9pt)[Steuernummer: #inputs.org_tax_number]

#v(0.5cm)
#text(size: 14pt, weight: "bold")[Aufstellung über das Vereinsvermögen]
#linebreak()
#text(size: 10pt)[zum 31.12.#inputs.year]

#v(0.5cm)
#line(length: 100%, stroke: 0.5pt)
#v(0.3cm)

// ── Table ────────────────────────────────────────────────────────────────────

#let arow(pos, label, amount, bold: false) = (
  text(weight: if bold { "bold" } else { "regular" })[#pos],
  text(weight: if bold { "bold" } else { "regular" })[#label],
  align(right, text(weight: if bold { "bold" } else { "regular" })[#amount]),
)

#table(
  columns: (2cm, 1fr, 3cm),
  stroke: (x, y) => (bottom: 0.4pt + luma(200)),
  fill: (_, y) => if y == 0 { luma(230) } else if calc.even(y) { luma(248) } else { none },
  inset: (x: 5pt, y: 4pt),

  text(weight: "bold")[Pos.],
  text(weight: "bold")[Position],
  align(right, text(weight: "bold")[Betrag (EUR)]),

  // ── A. Aktiva ──────────────────────────────────────────────────────────────
  table.cell(colspan: 3, text(weight: "bold")[A. Aktiva (Vermögenswerte)]),
  ..arow("A.1", "Bankkonto (Girokonto)", inputs.bank_balance),
  ..arow("A.2", "Bankkonto (Sparkonto / Rücklage)", zero),
  ..arow("A.3", "Kassenbestand (Barvermögen)", zero),
  ..arow("A.4", "Forderungen", zero),
  ..arow("A.5", "Sachanlagen / Inventar (Zeitwert)", inputs.inventory_value),
  ..arow("A.6", "Sonstige Vermögenswerte", zero),
  ..arow("", "Summe Aktiva", inputs.total_assets, bold: true),

  // ── B. Passiva ─────────────────────────────────────────────────────────────
  table.cell(colspan: 3, text(weight: "bold")[B. Passiva (Verbindlichkeiten)]),
  ..arow("B.7", "Verbindlichkeiten aus Lieferungen und Leistungen", zero),
  ..arow("B.8", "Sonstige Verbindlichkeiten / Rückstellungen", zero),
  ..arow("", "Summe Passiva", zero, bold: true),

  // ── Vereinsvermögen ────────────────────────────────────────────────────────
  table.cell(colspan: 3, []),
  ..arow("", "Vereinsvermögen (Aktiva − Passiva)", inputs.total_assets, bold: true),

  table.cell(colspan: 3, text(size: 9pt, fill: luma(100))[
    davon: Rücklagen gem. § 62 Abs. 1 Nr. 1 AO: #zero #h(1cm)
    davon: freie Rücklagen gem. § 62 Abs. 1 Nr. 3 AO: #zero
  ]),
)

#v(0.8cm)
#text(size: 9pt, fill: luma(100))[Erstellt am: #inputs.generated]

#v(1.5cm)
#line(length: 8cm, stroke: 0.5pt)
#linebreak()
#text(size: 9pt)[Ort, Datum / Unterschrift Vorstand]
