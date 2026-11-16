((script_element
  (raw_text) @injection.content)
 (#set! injection.language "typescript"))

((style_element
  (raw_text) @injection.content)
 (#set! injection.language "css"))

((interpolation
  (raw_text) @injection.content)
 (#set! injection.language "typescript"))

((directive_attribute
  (quoted_attribute_value
    (attribute_value) @injection.content))
 (#set! injection.language "typescript"))

((directive_attribute
  (attribute_value) @injection.content)
 (#set! injection.language "typescript"))
