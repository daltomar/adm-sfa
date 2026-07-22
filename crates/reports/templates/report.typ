// adm-sfa annual report template
// Data is injected via sys.inputs (see reports/pdf.rs)
#import sys: inputs

#set document(title: "adm-sfa Report")
#set page(paper: "a4", margin: 2cm)
#set text(size: 10pt)
#show heading.where(level: 1): it => {
  set text(size: 14pt, weight: "bold")
  v(0.3cm)
  it
  v(0.1cm)
  line(length: 100%, stroke: 0.5pt)
  v(0.1cm)
}
#show heading.where(level: 2): it => {
  set text(size: 11pt, weight: "bold")
  v(0.3cm)
  it
  v(0.05cm)
}

// ── Title block ──────────────────────────────────────────────────────────────

#text(size: 18pt, weight: "bold")[adm-sfa — Annual Report]

#v(0.3cm)

#let from = inputs.date_from
#let to   = inputs.date_to

#if from != "" or to != "" [
  #text(size: 10pt)[Period: #if from != "" { from } else { "–" } – #if to != "" { to } else { "(today)" }]
  #linebreak()
]

#text(size: 9pt, fill: luma(100))[Generated: #inputs.generated]

#v(0.6cm)
#line(length: 100%, stroke: 1pt)
#v(0.4cm)

// ── Sections ─────────────────────────────────────────────────────────────────

#for section in inputs.sections [
  == #section.title

  #if section.rows.len() == 0 [
    #text(style: "italic", fill: luma(120))[No data in the selected range.]
  ] else [
    #table(
      columns: section.headers.len(),
      stroke: (x, y) => if y == 0 { none } else { (bottom: 0.4pt + luma(200)) },
      fill:   (_, y) => if y == 0 { luma(230) } else if calc.odd(y) { luma(248) } else { none },
      inset:  (x: 6pt, y: 4pt),
      ..section.headers.map(h => text(weight: "bold")[#h]),
      ..section.rows.flatten().map(c => [#c]),
    )
  ]

  #v(0.5cm)
]
