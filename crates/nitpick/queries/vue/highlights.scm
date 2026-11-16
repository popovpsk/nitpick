(tag_name) @tag
(erroneous_end_tag_name) @tag
(doctype) @tag.doctype
(comment) @comment
(entity) @string.special

(attribute
  (attribute_name) @attribute)
(attribute
  (attribute_value) @string)
(attribute
  (quoted_attribute_value) @string)

(directive_name) @keyword
(directive_value) @attribute
(dynamic_directive_inner_value) @variable
(directive_modifier) @attribute
(directive_attribute
  (quoted_attribute_value
    [
      "\""
      "'"
    ] @string))

[
  "{{"
  "}}"
] @punctuation.special

[
  "<"
  ">"
  "</"
  "/>"
  "<!"
] @punctuation.bracket

[
  "["
  "]"
] @punctuation.bracket

[
  ":"
  "@"
  "#"
  "."
] @punctuation.delimiter

"=" @operator
