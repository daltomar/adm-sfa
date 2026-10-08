// Combined annual statutory report — all four documents in one PDF.
// Inputs: same dict as the individual templates; all fields present.
// See reports/src/statutory.rs :: build_dict
#import sys: inputs

#set document(title: "Jahresbericht " + inputs.year)
#set page(paper: "a4", margin: 2cm)
#set text(size: 10pt, lang: "de")
#show heading: it => {
  v(0.35cm)
  text(size: 11pt, weight: "bold")[#it.body]
  v(0.1cm)
  line(length: 100%, stroke: 0.4pt + luma(180))
  v(0.1cm)
}

// ─────────────────────────────────────────────────────────────────────────────
// DOKUMENT 1: Aufstellung über sämtliche Einnahmen und Ausgaben
// ─────────────────────────────────────────────────────────────────────────────

#let zero = "0,00"
#let dash = "—"

#text(size: 13pt, weight: "bold")[#inputs.org_name]
#linebreak()
#text(size: 9pt)[#inputs.org_address]
#linebreak()
#text(size: 9pt)[Steuernummer: #inputs.org_tax_number]

#v(0.5cm)
#text(size: 14pt, weight: "bold")[Aufstellung über sämtliche Einnahmen und Ausgaben]
#linebreak()
#text(size: 10pt)[Geschäftsjahr 01.01.#inputs.year – 31.12.#inputs.year]

#v(0.4cm)
#line(length: 100%, stroke: 0.5pt)
#v(0.2cm)

#let erow(pos, label, income, expense, bold: false) = (
  text(weight: if bold {"bold"} else {"regular"})[#pos],
  text(weight: if bold {"bold"} else {"regular"})[#label],
  align(right, text(weight: if bold {"bold"} else {"regular"})[#income]),
  align(right, text(weight: if bold {"bold"} else {"regular"})[#expense]),
)

#table(
  columns: (2cm, 1fr, 3cm, 3cm),
  stroke: (x, y) => (bottom: 0.4pt + luma(200)),
  fill: (_, y) => if y == 0 { luma(230) } else if calc.even(y) { luma(248) } else { none },
  inset: (x: 5pt, y: 4pt),
  text(weight: "bold")[Pos.], text(weight: "bold")[Position],
  align(right, text(weight: "bold")[Einnahmen (EUR)]),
  align(right, text(weight: "bold")[Ausgaben (EUR)]),
  table.cell(colspan: 4, text(weight: "bold", style: "italic")[Ideeller Bereich]),
  ..erow("1.", "Mitgliedsbeiträge", zero, dash),
  ..erow("2.", "Spenden (allgemein)", inputs.donations, dash),
  ..erow("3.", "Spenden (zweckgebunden)", zero, dash),
  ..erow("4.", "Öffentliche Zuschüsse / Zuwendungen", zero, dash),
  ..erow("5.", "Sonstige Einnahmen (Eigenleistungen)", inputs.self_funding, dash),
  ..erow("6.", "Projektausgaben", dash, inputs.project_expenses),
  ..erow("7.", "Personalkosten", dash, zero),
  ..erow("8.", "Sach- und Verwaltungskosten", dash, zero),
  ..erow("9.", "Raum-/Mietkosten und sonstige Ausgaben", dash, zero),
  ..erow("", "Zwischensumme 1 (Ideeller Bereich)", inputs.income_total, inputs.expense_total, bold: true),
  table.cell(colspan: 4, text(weight: "bold", style: "italic")[Vermögensverwaltung]),
  ..erow("10.", "Zinsen und ähnliche Einnahmen", zero, dash),
  ..erow("11.", "Miet- und Pachteinnahmen", zero, dash),
  ..erow("12.", "Sonstige Vermögenserträge", zero, dash),
  ..erow("13.", "Ausgaben der Vermögensverwaltung", dash, zero),
  ..erow("", "Zwischensumme 2 (Vermögensverwaltung)", zero, zero, bold: true),
  table.cell(colspan: 4, text(weight: "bold", style: "italic")[Zweckbetrieb]),
  ..erow("14.", "Einnahmen aus Zweckbetrieben", zero, dash),
  ..erow("15.", "Ausgaben für Zweckbetriebe", dash, zero),
  ..erow("", "Zwischensumme 3 (Zweckbetrieb)", zero, zero, bold: true),
  table.cell(colspan: 4, text(weight: "bold", style: "italic")[Steuerpflichtiger wirtschaftlicher Geschäftsbetrieb]),
  ..erow("16.", "Einnahmen aus wirtschaftlichem Geschäftsbetrieb", zero, dash),
  ..erow("17.", "Ausgaben für wirtschaftlichen Geschäftsbetrieb", dash, zero),
  ..erow("", "Zwischensumme 4 (Wirtschaftlicher Geschäftsbetrieb)", zero, zero, bold: true),
  table.cell(colspan: 4, []),
  ..erow("", [*Gesamtsumme Einnahmen / Ausgaben*], text(weight: "bold")[#inputs.income_total], text(weight: "bold")[#inputs.expense_total]),
  ..erow("", [*Überschuss / Fehlbetrag*], text(weight: "bold")[#inputs.surplus], dash),
)

#v(0.5cm)
#text(size: 9pt, fill: luma(100))[Erstellt am: #inputs.generated]
#v(1cm)
#line(length: 8cm, stroke: 0.5pt)
#linebreak()
#text(size: 9pt)[Ort, Datum / Unterschrift Vorstand]

// ─────────────────────────────────────────────────────────────────────────────
// DOKUMENT 2: Aufstellung über das Vereinsvermögen
// ─────────────────────────────────────────────────────────────────────────────

#pagebreak()

#text(size: 13pt, weight: "bold")[#inputs.org_name]
#linebreak()
#text(size: 9pt)[#inputs.org_address]
#linebreak()
#text(size: 9pt)[Steuernummer: #inputs.org_tax_number]

#v(0.5cm)
#text(size: 14pt, weight: "bold")[Aufstellung über das Vereinsvermögen]
#linebreak()
#text(size: 10pt)[zum 31.12.#inputs.year]

#v(0.4cm)
#line(length: 100%, stroke: 0.5pt)
#v(0.2cm)

#let vrow(pos, label, amount, bold: false) = (
  text(weight: if bold {"bold"} else {"regular"})[#pos],
  text(weight: if bold {"bold"} else {"regular"})[#label],
  align(right, text(weight: if bold {"bold"} else {"regular"})[#amount]),
)

#table(
  columns: (2cm, 1fr, 3cm),
  stroke: (x, y) => (bottom: 0.4pt + luma(200)),
  fill: (_, y) => if y == 0 { luma(230) } else if calc.even(y) { luma(248) } else { none },
  inset: (x: 5pt, y: 4pt),
  text(weight: "bold")[Pos.], text(weight: "bold")[Position],
  align(right, text(weight: "bold")[Betrag (EUR)]),
  table.cell(colspan: 3, text(weight: "bold")[A. Aktiva (Vermögenswerte)]),
  ..vrow("A.1", "Bankkonto (Girokonto)", inputs.bank_balance),
  ..vrow("A.2", "Bankkonto (Sparkonto / Rücklage)", zero),
  ..vrow("A.3", "Kassenbestand (Barvermögen)", zero),
  ..vrow("A.4", "Forderungen", zero),
  ..vrow("A.5", "Sachanlagen / Inventar (Zeitwert)", inputs.inventory_value),
  ..vrow("A.6", "Sonstige Vermögenswerte", zero),
  ..vrow("", "Summe Aktiva", inputs.total_assets, bold: true),
  table.cell(colspan: 3, text(weight: "bold")[B. Passiva (Verbindlichkeiten)]),
  ..vrow("B.7", "Verbindlichkeiten aus Lieferungen und Leistungen", zero),
  ..vrow("B.8", "Sonstige Verbindlichkeiten / Rückstellungen", zero),
  ..vrow("", "Summe Passiva", zero, bold: true),
  table.cell(colspan: 3, []),
  ..vrow("", "Vereinsvermögen (Aktiva − Passiva)", inputs.total_assets, bold: true),
  table.cell(colspan: 3, text(size: 9pt, fill: luma(100))[
    davon: Rücklagen gem. § 62 Abs. 1 Nr. 1 AO: #zero #h(1cm)
    davon: freie Rücklagen gem. § 62 Abs. 1 Nr. 3 AO: #zero
  ]),
)

#v(0.5cm)
#text(size: 9pt, fill: luma(100))[Erstellt am: #inputs.generated]
#v(1cm)
#line(length: 8cm, stroke: 0.5pt)
#linebreak()
#text(size: 9pt)[Ort, Datum / Unterschrift Vorstand]

// ─────────────────────────────────────────────────────────────────────────────
// DOKUMENT 3: Tätigkeitsbericht
// ─────────────────────────────────────────────────────────────────────────────

#pagebreak()

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
#v(0.3cm)
#line(length: 100%, stroke: 0.5pt)

= 1. Vereinszweck

Der Verein fördert den Sport (§ 52 Abs. 2 Nr. 21 AO), insbesondere den
Skateboardsport und die Skateboard-Kultur in Brasilien.

= 2. Durchgeführte Maßnahmen

#if inputs.outbound_events.len() > 0 [
  #table(
    columns: (2.5cm, 1fr, 2cm),
    stroke: (x, y) => (bottom: 0.4pt + luma(200)),
    fill: (_, y) => if y == 0 { luma(230) } else if calc.even(y) { luma(248) } else { none },
    inset: (x: 5pt, y: 4pt),
    text(weight: "bold")[Datum], text(weight: "bold")[Empfänger / Projekt],
    align(right, text(weight: "bold")[Artikel]),
    ..inputs.outbound_events.map(e => (e.date, e.recipient, align(right)[#e.item_count])).flatten(),
  )
  #v(0.2cm)
]

#for para in inputs.tb_activities_paras [ #para #v(0.2cm) ]

= 3. Sonstige Tätigkeiten

#for para in inputs.tb_continuous_paras [ #para #v(0.2cm) ]

= 4. Ausblick #str(int(inputs.year) + 1)

#for para in inputs.tb_outlook_paras [ #para #v(0.2cm) ]

= 5. Vorstand und Mitgliederversammlung

Im Berichtsjahr #inputs.year fand eine gemeinsame Vorstandssitzung und
Mitgliederversammlung statt.

#v(0.8cm)
#text(size: 9pt, fill: luma(100))[Erstellt am: #inputs.generated]
#v(1cm)
#grid(columns: (1fr, 1fr), gutter: 1cm,
  [#line(length: 100%, stroke: 0.5pt) \ #text(size: 9pt)[Ort, Datum / Unterschrift Vorstand]],
  [#line(length: 100%, stroke: 0.5pt) \ #text(size: 9pt)[Unterschrift Kassenwart]],
)

// ─────────────────────────────────────────────────────────────────────────────
// DOKUMENT 4: Protokoll der Mitgliederversammlung
// ─────────────────────────────────────────────────────────────────────────────

#pagebreak()

#text(size: 13pt, weight: "bold")[#inputs.org_name]

#v(0.4cm)
#text(size: 14pt, weight: "bold")[Protokoll der gemeinsamen Vorstandssitzung und Mitgliederversammlung]
#v(0.3cm)
#line(length: 100%, stroke: 0.5pt)
#v(0.3cm)

#grid(columns: (3cm, 1fr), gutter: 4pt,
  text(weight: "bold")[Datum:], inputs.meeting_date,
  text(weight: "bold")[Zeit:], [#inputs.meeting_time_from Uhr – #inputs.meeting_time_to Uhr],
  text(weight: "bold")[Ort:], inputs.org_address,
  text(weight: "bold")[Stimmberechtigte:], [2 (beide anwesend, Versammlung ist beschlussfähig)],
)

#v(0.4cm)
= Tagesordnung

+ Begrüßung und Feststellung der Beschlussfähigkeit
+ Genehmigung der Tagesordnung
+ Tätigkeitsbericht des Vorstands – Geschäftsjahr #inputs.year
+ Kassenbericht – Geschäftsjahr #inputs.year
+ Entlastung des Vorstands
+ Planung der Tätigkeiten #inputs.next_year
+ Verschiedenes

= TOP 1–2

Begrüßung und Beschlussfähigkeit festgestellt. Tagesordnung einstimmig genehmigt.

= TOP 3 – Tätigkeitsbericht

#if inputs.outbound_events.len() > 0 [
  #table(
    columns: (2.5cm, 1fr, 2cm),
    stroke: (x, y) => (bottom: 0.4pt + luma(200)),
    fill: (_, y) => if y == 0 { luma(230) } else if calc.even(y) { luma(248) } else { none },
    inset: (x: 5pt, y: 4pt),
    text(weight: "bold")[Datum], text(weight: "bold")[Empfänger / Projekt],
    align(right, text(weight: "bold")[Artikel]),
    ..inputs.outbound_events.map(e => (e.date, e.recipient, align(right)[#e.item_count])).flatten(),
  )
  #v(0.2cm)
]

= TOP 4 – Kassenbericht

#table(
  columns: (1fr, 3cm),
  stroke: (x, y) => (bottom: 0.4pt + luma(200)),
  fill: (_, y) => if y == 0 { luma(230) } else { none },
  inset: (x: 5pt, y: 4pt),
  text(weight: "bold")[Position], align(right, text(weight: "bold")[EUR]),
  [Einnahmen (Spenden)], align(right)[#inputs.income_total],
  [Ausgaben (Skateboard-Material zur Sachspende)], align(right)[#inputs.expense_total],
  [*Kontostand zum 31.12.#inputs.year*], align(right, text(weight: "bold")[#inputs.bank_balance]),
)

= TOP 5 – Entlastung

Einstimmige Entlastung. Abstimmungsergebnis: 2 Ja, 0 Nein, 0 Enthaltungen.

= TOP 6 – Tätigkeiten #inputs.next_year

#for para in inputs.pk_activities_paras [ #para #v(0.2cm) ]

Abstimmungsergebnis: 2 Ja, 0 Nein, 0 Enthaltungen.

= TOP 7 – Verschiedenes

#if inputs.pk_decisions_paras.len() > 0 [
  #for para in inputs.pk_decisions_paras [ #para #v(0.2cm) ]
] else [Keine weiteren Punkte.]

#v(0.8cm)
#text(size: 9pt, fill: luma(100))[Erstellt am: #inputs.generated]
#v(1cm)
#grid(columns: (1fr, 1fr), gutter: 1cm,
  [#line(length: 100%, stroke: 0.5pt) \ #text(size: 9pt)[Ort, Datum / Unterschrift Vorstand]],
  [#line(length: 100%, stroke: 0.5pt) \ #text(size: 9pt)[Unterschrift Kassenwart]],
)
