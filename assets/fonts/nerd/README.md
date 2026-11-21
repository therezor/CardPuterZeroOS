# Nerd Font icons

`SymbolsNerdFont-icons.ttf` is a subset of `SymbolsNerdFont-Regular.ttf` from
Nerd Fonts v3.5.1 (`NerdFontsSymbolsOnly.tar.xz`). It keeps the code points
in `icons.txt`: the icons in yazi 26.9.1's dark and light themes, without the
Powerline range (U+E0A0 to U+E0D7), which the terminal draws itself.
`gh0st-ui/build.rs` bakes every glyph in it at each Terminus size.

To add icons, add their code points to `icons.txt` and subset again:

```sh
pyftsubset SymbolsNerdFont-Regular.ttf --unicodes="$(paste -sd, icons.txt)" \
  --output-file=SymbolsNerdFont-icons.ttf --no-hinting --desubroutinize \
  --layout-features= --notdef-outline --name-IDs='*' --name-languages='*'
```

The font is MIT (`LICENSE`). The icon sets in it keep their own licences:

| Set | Upstream | Licence |
|---|---|---|
| Codicons | github.com/microsoft/vscode-codicons | CC BY 4.0 |
| Devicons | github.com/devicons/devicon | MIT |
| Font Awesome | github.com/FortAwesome/Font-Awesome | CC BY 4.0 |
| Font Awesome Extension | github.com/AndreLZGava/font-awesome-extension | MIT |
| Font Logos | github.com/lukas-w/font-logos | Unlicense |
| Material Design | github.com/Templarian/MaterialDesign-Font | Apache 2.0 |
| Octicons | github.com/primer/octicons | MIT |
| Seti UI | github.com/jesseweed/seti-ui | MIT |
| Pomicons | github.com/gabrielelana/pomicons | OFL 1.1 |
| Powerline Extra | github.com/ryanoasis/powerline-extra-symbols | MIT |

Logos are trademarks of their owners.
