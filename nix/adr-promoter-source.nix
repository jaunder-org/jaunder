# The Python package filter and live-checkout admission share this authority.
relative: type:
if type == "directory" then
  relative == "tools/adr-promoter" || builtins.match "tools/adr-promoter/.*" relative != null
else
  relative == "tools/adr-promoter/pyproject.toml"
  || builtins.match "tools/adr-promoter/(adr_promoter|tests)/.*[.]py" relative != null
