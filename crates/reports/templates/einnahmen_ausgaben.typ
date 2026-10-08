// Aufstellung über sämtliche Einnahmen und Ausgaben
// Inputs: see reports/src/statutory.rs :: build_dict
#import sys: inputs

#set document(title: "Einnahmen und Ausgaben")
#set page(paper: "a4", margin: 2cm)
#set text(size: 10pt, lang: "de")

#let zero = "0,00"
#let dash = "—"

// ── Header ───────────────────────────────────────────────────────────────────

#text(size: 13pt, weight: "bold")[#inputs.org_name]
#linebreak()
#text(size: 9pt)[#inputs.org_address]
#linebreak()
#text(size: 9pt)[Steuernummer: #inputs.org_tax_number]

#v(0.5cm)
#text(size: 14pt, weight: "bold")[Aufstellung über sämtliche Einnahmen und Ausgaben]
#linebreak()
#text(size: 10pt)[Geschäftsjahr 01.01.#inputs.year – 31.12.#inputs.year]

#v(0.5cm)
#line(length: 100%, stroke: 0.5pt)
#v(0.3cm)

// ── Table helper ─────────────────────────────────────────────────────────────

#let row(pos, label, income, expense) = (
  text(weight: if pos == "" { "bold" } else { "regular" })[#pos],
  text(weight: if pos == "" { "bold" } else { "regular" })[#label],
  align(right, text(weight: if pos == "" { "bold" } else { "regular" })[#income]),
  align(right, text(weight: if pos == "" { "bold" } else { "regular" })[#expense]),
)

#table(
  columns: (2cm, 1fr, 3cm, 3cm),
  stroke: (x, y) => (bottom: 0.4pt + luma(200)),
  fill: (_, y) => if y == 0 { luma(230) } else if calc.even(y) { luma(248) } else { none },
  inset: (x: 5pt, y: 4pt),

  // Header row
  text(weight: "bold")[Pos.], text(weight: "bold")[Position],
  align(right, text(weight: "bold")[Einnahmen (EUR)]),
  align(right, text(weight: "bold")[Ausgaben (EUR)]),

  // ── Ideeller Bereich ───────────────────────────────────────────────────────
  table.cell(colspan: 4, text(weight: "bold", style: "italic")[Ideeller Bereich]),
  ..row("1.", "Mitgliedsbeiträge", zero, dash),
  ..row("2.", "Spenden (allgemein)", inputs.donations, dash),
  ..row("3.", "Spenden (zweckgebunden)", zero, dash),
  ..row("4.", "Öffentliche Zuschüsse / Zuwendungen", zero, dash),
  ..row("5.", "Sonstige Einnahmen (Eigenleistungen)", inputs.self_funding, dash),
  ..row("6.", "Projektausgaben", dash, inputs.project_expenses),
  ..row("7.", "Personalkosten", dash, zero),
  ..row("8.", "Sach- und Verwaltungskosten", dash, zero),
  ..row("9.", "Raum-/Mietkosten und sonstige Ausgaben", dash, zero),
  ..row("", "Zwischensumme 1 (Ideeller Bereich)", inputs.income_total, inputs.expense_total),

  // ── Vermögensverwaltung ────────────────────────────────────────────────────
  table.cell(colspan: 4, text(weight: "bold", style: "italic")[Vermögensverwaltung]),
  ..row("10.", "Zinsen und ähnliche Einnahmen", zero, dash),
  ..row("11.", "Miet- und Pachteinnahmen", zero, dash),
  ..row("12.", "Sonstige Vermögenserträge", zero, dash),
  ..row("13.", "Ausgaben der Vermögensverwaltung", dash, zero),
  ..row("", "Zwischensumme 2 (Vermögensverwaltung)", zero, zero),

  // ── Zweckbetrieb ──────────────────────────────────────────────────────────
  table.cell(colspan: 4, text(weight: "bold", style: "italic")[Zweckbetrieb]),
  ..row("14.", "Einnahmen aus Zweckbetrieben", zero, dash),
  ..row("15.", "Ausgaben für Zweckbetriebe", dash, zero),
  ..row("", "Zwischensumme 3 (Zweckbetrieb)", zero, zero),

  // ── Wirtschaftlicher Geschäftsbetrieb ─────────────────────────────────────
  table.cell(colspan: 4,
    text(weight: "bold", style: "italic")[Steuerpflichtiger wirtschaftlicher Geschäftsbetrieb]),
  ..row("16.", "Einnahmen aus wirtschaftlichem Geschäftsbetrieb", zero, dash),
  ..row("17.", "Ausgaben für wirtschaftlichen Geschäftsbetrieb", dash, zero),
  ..row("", "Zwischensumme 4 (Wirtschaftlicher Geschäftsbetrieb)", zero, zero),

  // ── Gesamtsumme ───────────────────────────────────────────────────────────
  table.cell(colspan: 4, []),
  ..row("", [*Gesamtsumme Einnahmen / Ausgaben*], text(weight: "bold")[#inputs.income_total],
    text(weight: "bold")[#inputs.expense_total]),
  ..row("", [*Überschuss / Fehlbetrag*], text(weight: "bold")[#inputs.surplus], dash),
)

#v(0.8cm)
#text(size: 9pt, fill: luma(100))[Erstellt am: #inputs.generated]

#v(1.5cm)
#line(length: 8cm, stroke: 0.5pt)
#linebreak()
#text(size: 9pt)[Ort, Datum / Unterschrift Vorstand]
