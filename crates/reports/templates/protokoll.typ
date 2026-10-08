// Protokoll der gemeinsamen Vorstandssitzung und Mitgliederversammlung
// Inputs: see reports/src/statutory.rs :: build_dict
#import sys: inputs

#set document(title: "Protokoll Mitgliederversammlung")
#set page(paper: "a4", margin: 2cm)
#set text(size: 10pt, lang: "de")
#show heading: it => {
  v(0.35cm)
  text(size: 11pt, weight: "bold")[#it.body]
  v(0.1cm)
}

// ── Header ───────────────────────────────────────────────────────────────────

#text(size: 13pt, weight: "bold")[#inputs.org_name]

#v(0.4cm)
#text(size: 14pt, weight: "bold")[Protokoll der gemeinsamen Vorstandssitzung und Mitgliederversammlung]
#v(0.3cm)
#line(length: 100%, stroke: 0.5pt)
#v(0.3cm)

#grid(
  columns: (3cm, 1fr),
  gutter: 4pt,
  text(weight: "bold")[Datum:], inputs.meeting_date,
  text(weight: "bold")[Zeit:],
    [#inputs.meeting_time_from Uhr – #inputs.meeting_time_to Uhr],
  text(weight: "bold")[Ort:], inputs.org_address,
  text(weight: "bold")[Stimmberechtigte:],
    [2 (beide anwesend, Versammlung ist beschlussfähig)],
)

#v(0.5cm)

= Tagesordnung

+ Begrüßung und Feststellung der Beschlussfähigkeit
+ Genehmigung der Tagesordnung
+ Tätigkeitsbericht des Vorstands – Geschäftsjahr #inputs.year
+ Kassenbericht – Geschäftsjahr #inputs.year
+ Entlastung des Vorstands
+ Planung der Tätigkeiten #inputs.next_year
+ Verschiedenes

= TOP 1 – Begrüßung und Beschlussfähigkeit

Die Versammlung wird eröffnet. Alle stimmberechtigten Mitglieder sind anwesend.
Die Versammlung ist beschlussfähig.

= TOP 2 – Genehmigung der Tagesordnung

Die Tagesordnung wird ohne Änderungen einstimmig genehmigt.

= TOP 3 – Tätigkeitsbericht des Vorstands

Der Vorstand berichtet über die Aktivitäten im Geschäftsjahr #inputs.year.

#if inputs.outbound_events.len() > 0 [
  Folgende Maßnahmen wurden durchgeführt:

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
  #v(0.2cm)
]

Der vollständige Tätigkeitsbericht ist als Anlage beigefügt.

= TOP 4 – Kassenbericht

Der Kassenwart legt den Kassenbericht für das Geschäftsjahr #inputs.year vor:

#table(
  columns: (1fr, 3cm),
  stroke: (x, y) => (bottom: 0.4pt + luma(200)),
  fill: (_, y) => if y == 0 { luma(230) } else { none },
  inset: (x: 5pt, y: 4pt),
  text(weight: "bold")[Position], align(right, text(weight: "bold")[EUR]),
  [Einnahmen (Spenden)], align(right)[#inputs.income_total],
  [Ausgaben (Anschaffung Skateboard-Material zur Sachspende)],
    align(right)[#inputs.expense_total],
  [*Kontostand zum 31.12.#inputs.year*], align(right, text(weight: "bold")[#inputs.bank_balance]),
)

#v(0.2cm)
Die vollständige Aufstellung über Einnahmen/Ausgaben und die Vermögensaufstellung
sind als Anlage beigefügt.

= TOP 5 – Entlastung des Vorstands

Der Vorstand wird für das Geschäftsjahr #inputs.year einstimmig entlastet. \
Abstimmungsergebnis: 2 Ja, 0 Nein, 0 Enthaltungen.

= TOP 6 – Planung der Tätigkeiten #inputs.next_year

#for para in inputs.pk_activities_paras [
  #para
  #v(0.2cm)
]

Abstimmungsergebnis: 2 Ja, 0 Nein, 0 Enthaltungen.

= TOP 7 – Verschiedenes

#if inputs.pk_decisions_paras.len() > 0 [
  #for para in inputs.pk_decisions_paras [
    #para
    #v(0.2cm)
  ]
] else [
  Keine weiteren Punkte.
]

= Schluss der Versammlung

Die Versammlung wird um #inputs.meeting_time_to Uhr geschlossen.

#v(1cm)
#text(size: 9pt, fill: luma(100))[Erstellt am: #inputs.generated]

#v(1.5cm)
#grid(columns: (1fr, 1fr), gutter: 1cm,
  [
    #line(length: 100%, stroke: 0.5pt) \
    #text(size: 9pt)[Ort, Datum / Unterschrift Vorstand (Protokollführer)]
  ],
  [
    #line(length: 100%, stroke: 0.5pt) \
    #text(size: 9pt)[Unterschrift Kassenwart]
  ]
)
