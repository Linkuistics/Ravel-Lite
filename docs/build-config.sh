# Per-project config consumed by scripts/build-docs.sh. Copy this file
# into another project that wants the same doc-build pattern and tweak
# only these values — scripts/build-docs.sh and docs/templates/ stay
# universal.

SITE_NAME="Linkuistics"
SITE_TAGLINE="The language of the web"

PROJECT_NAME="Ravel-Lite"
PROJECT_SLUG="ravel-lite"

# Relative URLs as they appear from a built chapter page at
# projects/<slug>/*.html. Keep them anchored to the deployed site layout,
# not to paths inside this repo.
HOME_HREF="../../index.html"
LOGO_HREF="../../img/logo.svg"
CSS_HREF="../../css/style.css"
BACK_LINK_HREF="../${PROJECT_SLUG}.html"
BACK_LINK_TEXT="${PROJECT_NAME}"

# Where to write the built .html files. Relative paths are resolved from
# the repo root (the parent of docs/). For Linkuistics projects this is
# the sibling checkout of www.linkuistics.com.
OUTPUT_DIR="../www.linkuistics.com/projects/ravel-lite"
