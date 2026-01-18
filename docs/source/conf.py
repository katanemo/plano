# Configuration file for the Sphinx documentation builder.
#
# For the full list of built-in configuration values, see the documentation:
# https://www.sphinx-doc.org/en/master/usage/configuration.html

# -- Project information -----------------------------------------------------
# https://www.sphinx-doc.org/en/master/usage/configuration.html#project-information
import os
import sys
from dataclasses import asdict

from sphinx.application import Sphinx
from sphinx.util.docfields import Field
from sphinxawesome_theme import ThemeOptions
from sphinxawesome_theme.postprocess import Icons

project = "Plano Docs"
copyright = "2025, Katanemo Labs, Inc"
author = "Katanemo Labs, Inc"
release = " v0.4.3"

# -- General configuration ---------------------------------------------------
# https://www.sphinx-doc.org/en/master/usage/configuration.html#general-configuration


root_doc = "index"

nitpicky = True
add_module_names = False

# -- General configuration ---------------------------------------------------
extensions = [
    "sphinx.ext.autodoc",
    "sphinx.ext.intersphinx",
    "sphinx.ext.extlinks",
    "sphinx.ext.viewcode",
    "sphinx_sitemap",
    "sphinx_design",
    # Local extensions
    "llms_txt",
]

# Paths that contain templates, relative to this directory.
templates_path = ["_templates"]

# List of patterns, relative to source directory, that match files and directories
# to ignore when looking for source files.
exclude_patterns = ["_build", "Thumbs.db", ".DS_Store"]

# Allow importing extensions from docs/source/_ext (robust to current working directory)
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "_ext")))


# -- Options for HTML output -------------------------------------------------
html_theme = "sphinxawesome_theme"  # You can change the theme to 'sphinx_rtd_theme' or another of your choice.
html_title = project + release
html_permalinks_icon = Icons.permalinks_icon
html_favicon = "_static/favicon.ico"
html_logo = "_static/favicon.ico"  # Specify the path to the logo image file (make sure the logo is in the _static directory)
html_last_updated_fmt = ""
html_use_index = False  # Don't create index
html_domain_indices = False  # Don't need module indices
html_copy_source = False  # Don't need sources
html_show_sphinx = False


html_baseurl = "./docs"

html_sidebars = {
    "**": [
        "analytics.html",
        "sidebar_main_nav_links.html",
        "sidebar_toc.html",
    ]
}

theme_options = ThemeOptions(
    show_breadcrumbs=True,
    awesome_external_links=True,
    extra_header_link_icons={
        "repository on GitHub": {
            "link": "https://github.com/katanemo/plano",
            "icon": (
                '<svg height="26px" style="margin-top:-2px;display:inline" '
                'viewBox="0 0 45 44" '
                'fill="currentColor" xmlns="http://www.w3.org/2000/svg">'
                '<path fill-rule="evenodd" clip-rule="evenodd" '
                'd="M22.477.927C10.485.927.76 10.65.76 22.647c0 9.596 6.223 17.736 '
                "14.853 20.608 1.087.2 1.483-.47 1.483-1.047 "
                "0-.516-.019-1.881-.03-3.693-6.04 "
                "1.312-7.315-2.912-7.315-2.912-.988-2.51-2.412-3.178-2.412-3.178-1.972-1.346.149-1.32.149-1.32 "  # noqa
                "2.18.154 3.327 2.24 3.327 2.24 1.937 3.318 5.084 2.36 6.321 "
                "1.803.197-1.403.759-2.36 "
                "1.379-2.903-4.823-.548-9.894-2.412-9.894-10.734 "
                "0-2.37.847-4.31 2.236-5.828-.224-.55-.969-2.759.214-5.748 0 0 "
                "1.822-.584 5.972 2.226 "
                "1.732-.482 3.59-.722 5.437-.732 1.845.01 3.703.25 5.437.732 "
                "4.147-2.81 5.967-2.226 "
                "5.967-2.226 1.185 2.99.44 5.198.217 5.748 1.392 1.517 2.232 3.457 "
                "2.232 5.828 0 "
                "8.344-5.078 10.18-9.916 10.717.779.67 1.474 1.996 1.474 4.021 0 "
                "2.904-.027 5.247-.027 "
                "5.96 0 .58.392 1.256 1.493 1.044C37.981 40.375 44.2 32.24 44.2 "
                '22.647c0-11.996-9.726-21.72-21.722-21.72" '
                'fill="currentColor"/></svg>'
            ),
        },
    },
)

html_theme_options = asdict(theme_options)

# Add any paths that contain custom static files (such as style sheets) here,
# relative to this directory. They are copied after the builtin static files,
# so a file named "default.css" will overwrite the builtin "default.css".
html_static_path = ["_static"]
html_css_files = ["css/custom.css"]

pygments_style = "lovelace"
pygments_style_dark = "github-dark"

sitemap_url_scheme = "{link}"
# Add this configuration at the bottom of your conf.py

html_context = {
    "google_analytics_id": "G-EH2VW19FXE",  # Replace with your Google Analytics tracking ID
}

templates_path = ["_templates"]


# -- Register a :confval: interpreted text role ----------------------------------
def setup(app: Sphinx) -> None:
    """Register the ``confval`` role and directive.

    This allows to declare theme options as their own object
    for styling and cross-referencing.
    """
    app.add_object_type(
        "confval",
        "confval",
        objname="configuration parameter",
        doc_field_types=[
            Field(
                "default",
                label="default",
                has_arg=True,
                names=("default",),
                bodyrolename="class",
            )
        ],
    )
