# SPEC — Inkarchy

Référence produit. À lire **avant** toute modification. Si une demande contredit ce fichier, le signaler avant de coder.

## But
Inkarchy : app native Rust (egui/eframe) de notes manuscrites au stylet, dans l'esprit Omarchy : rapide, minimaliste, visuelle, locale. Inspirée de Samsung Notes / Apple Notes, mais chaque élément est un objet de papeterie (étagère, chemise, pupitre), pas une carte Material. Ancien nom : Cahier.

## Plateforme cible
- Linux, Wayland d'abord (Hyprland / Omarchy, Arch). X11 supporté par eframe.
- Écran tactile + stylet (tablet Wayland) **et** clavier/souris.
- Binaire `inkarchy`, app id `com.clouddown.inkarchy`, desktop `inkarchy.desktop`, Rust ≥ 1.89.
- Thème lu depuis `~/.local/state/omarchy/current/theme/` (`colors.toml`), police UI via `fc-match monospace`.

## Fonctionnalités
### Étagère (shelf)
- [x] Cahiers sur une grille de cases (trous autorisés), titre et icône emoji modifiables. Sélecteur emoji : onglets type iPhone, décodage progressif.
- [x] Roue d'actions autour du dossier : appui long au doigt (glisser puis lâcher pour agir), clic droit à la souris (clic pour agir). Actions : couleur, corbeille, marque.
- [x] Roue souris : ne se ferme qu'en sortant de la roue ; clic au centre dans la sous-roue couleurs = retour, pas fermeture.
- [x] Sélection multiple au rectangle (souris) ou appui long (doigt).
- [x] Corbeille : bouton en bas (emoji), rangée rouge de 10 cases ; glisser sur l'icône corbeille = jeter ; glisser dans une case seulement si la corbeille est ouverte ; glisser hors = restaurer ; bouton Empty visible quand elle est ouverte.
- [x] Recherche au centre, interface en anglais, mots professionnels. Filtre titre, texte tapé / stickies, et Unicode des PDF de fond (pas d’OCR, pas le manuscrit).
- [x] Panneau Help minimaliste : titre « Help », raccourcis clavier/souris à gauche, mains à droite ; texture Canson.
- [x] Défilement à deux doigts.
- [x] Cahiers d'exemple au premier lancement (*Mindset*, *Art direction*).

### Pupitre (page)
- [x] Outils : feutre, plume, crayon, surligneur, gommes trait/zone, lasso, texte, image, règle.
- [x] Papiers : blanc, ligné, carreaux, pointillé, millimétré, ardoise ; texture Canson (`assets/canson.jpg`).
- [x] Reconnaissance de formes en gardant le stylet immobile ~1 s.
- [x] Pages multiples : bouton Pages (onglets `+` verts et `−` rouge au centre de chaque page), modes Linked / Separate.
- [x] Bascule automatique souris ↔ stylet à l'approche du stylet (pas de bouton manuel).
- [x] Gestes tactiles : 1 doigt = déplacer papier/objet, appui court = sélection de trait, 2 doigts = pan/zoom, tap 2 doigts = annuler.
- [x] « Draw with finger » et type de gomme (trait / zone) dans le menu du cahier.
- [x] Appui long sur un stylo, ou la pastille d'encre, ouvre ensemble la soucoupe de couleur et le trait d'épaisseur.
- [x] Mise en page petits écrans (min 320×280), rester sur la zone cadrée au resize.

### Fichiers
- [x] Export PNG / PDF via un seul menu Download….
- [x] Ouvrir PDF (fond non éditable, annotations par-dessus, export PDF qui garde l'original), PNG/JPEG/WEBP/SVG, `.txt`/`.md` ≤ 64 KiB. *(PR #2, brouillon)*
- [x] « Open with » + deuxième instance qui renvoie les chemins à la fenêtre ouverte. *(PR #2)*
- [x] `install.sh` : binaire de release, `--from-source`, `--uninstall`, entrée desktop + icône. *(PR #1)*
- [x] CI : fmt, clippy `-D warnings`, tests ; section CHANGELOG obligatoire pour la version Cargo.toml ; push sur `main` avec version neuve → tag `v*` + release Linux. *(voir `docs/RELEASING.md`)*

### Demandé / pas fait
- [ ] Fusionner PR #1 puis PR #2 dans `main` et publier `v0.2.0`.
- [x] Nom de produit Inkarchy (dépôt CloudDown/inkarchy).
- [ ] Icônes de la roue fournies par l'utilisateur (à confirmer : déjà livrées ?).
- [ ] Raccourcis emoji/corbeille au clavier listés dans Help (à confirmer).
- [ ] Travail local non commité en cours (app.rs, pdf.rs, tests) : contenu à confirmer.

## Règles UX à ne jamais casser
1. Doigt et souris ont des comportements différents pour la roue (glisser-lâcher vs clic). Clic droit souris = roue, pas appui long.
2. La corbeille ne s'affiche pas toute seule pendant un glisser ; on vise l'icône.
3. L'app détecte seule stylet/main ; pas de bascule manuelle.
4. Minimalisme : pas de mots inutiles, pas de barre de défilement inutile, pas de fond derrière la corbeille.
5. Chrome = couleurs `Look` du thème Omarchy ; le papier garde son look ; corbeille en rouge du thème.
6. Glyphes vectoriels pour les actions de chrome ; atlas emoji pour stylos, lasso, corbeille, marques.
7. Ne jamais supprimer les proxys Wayland `allow(dead_code)` ni les impls Dispatch vides du tablet-pad (sinon stylet/pinch coupés).

## Données et sauvegarde
- Dossier : `~/.local/share/inkarchy` (ou `$INKARCHY_DATA` ; `$CAHIER_DATA` est un alias déprécié).
- Migration : au démarrage, si `~/.local/share/omacourses` existe et que le nouveau dossier n'existe pas, le renommer (copie si le rename échoue) et le journaliser.
- `library.json` : index étagère + corbeille (slots). `<id>/note.json` par cahier ; fichiers importés copiés dans ce dossier.
- Écriture atomique (`atomic_write` : fichier temporaire même dossier, fsync, rename). Un échec = toast, jamais silencieux.
- Mettre à la corbeille garde les fichiers ; seul « Empty » supprime.
- Compatibilité : ne jamais renommer champs JSON ni ids internes (`fab-corbeille`, `cahier-dock`, alias serde). Anciennes notes doivent toujours s'ouvrir.
- Aucun cloud, aucun compte, aucun réseau.

## Non-objectifs
- Cloud, synchro, comptes, collaboration.
- Second produit dans ce dépôt.
- Look Material / cartes génériques.
- Dépendance à Poppler/MuPDF/PDFium (hayro est intégré).

## Definition of Done
- `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test --offline` passent.
- Testé à la main au stylet/doigt **et** souris si l'UI change.
- Aucune case [x] ci-dessus régressée ; SPEC et CHANGELOG mis à jour si une fonctionnalité change.
- `docs/screens/` mis à jour seulement pour la fonctionnalité touchée ; README en anglais.
- Commit en français, une phrase concrète, une fonctionnalité par commit.

## Instructions pour l'IA
- Lire ce SPEC (et `AGENTS.md`) avant de répondre ou coder.
- Changer **uniquement** ce qui est demandé ; garder le reste tel quel.
- Montrer le diff avant d'appliquer un changement non trivial.
- Si une demande casse une règle UX ou une case [x], le dire d'abord.
- Ne pas inventer de fonctionnalité ; demander si c'est flou.
- Répondre en français.
