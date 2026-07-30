import re

with open("crates/markdown/src/view.rs", "r") as f:
    content = f.read()

# 1. Revert layout_snapshot to visible_line_snapshots
content = re.sub(
    r'layout_snapshot\(editor\.engine\(\)\),\n\s*before_click,',
    r'visible_line_snapshots(editor.engine()),\n                before_click,',
    content
)

# 2. Fix the source fixture to have only one empty line between paragraphs
# find the specific gap
content = content.replace(
    "机电、轨道、计算机应用、汽车检测等\n\n\n定向军士",
    "机电、轨道、计算机应用、汽车检测等\n\n定向军士"
)

with open("crates/markdown/src/view.rs", "w") as f:
    f.write(content)
