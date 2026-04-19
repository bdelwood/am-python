"""Sphinx configuration for am-python docs."""

project = "am-python"
extensions = ["autoapi.extension", "myst_parser"]

myst_enable_extensions = ["attrs_inline"]

autoapi_type = "python"
autoapi_dirs = [".."]
autoapi_file_patterns = ["am.pyi"]
autoapi_options = ["members", "undoc-members", "show-module-summary"]

source_suffix = {
    ".md": "markdown",
    ".rst": "restructuredtext",
}

exclude_patterns = ["_build", "Thumbs.db", ".DS_Store"]

html_theme = "furo"
html_title = "am-python"
