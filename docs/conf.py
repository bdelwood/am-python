"""Sphinx configuration for am-python docs."""

project = "am-python"
extensions = ["autoapi.extension", "myst_parser"]

myst_enable_extensions = ["attrs_inline"]

autoapi_type = "python"
autoapi_dirs = ["../python"]
autoapi_file_patterns = ["*.pyi", "*.py"]
autoapi_options = [
    "members",
    "undoc-members",
    "show-module-summary",
    "imported-members",
]

source_suffix = {
    ".md": "markdown",
    ".rst": "restructuredtext",
}

exclude_patterns = ["_build", "Thumbs.db", ".DS_Store"]

html_theme = "furo"
html_title = "am-python"
