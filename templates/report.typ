// adm-sfa report template — placeholder for typst-as-lib integration
#set document(title: "adm-sfa Report")
#set page(paper: "a4", margin: 2cm)
#set text(font: "Liberation Sans", size: 10pt)

= adm-sfa Report

#v(0.5cm)
Generated: #datetime.today().display()

#v(0.5cm)
#table(
  columns: (auto, auto, auto),
  stroke: 0.5pt,
  [*Date*], [*Description*], [*Amount*],
  [—], [No data], [—],
)
