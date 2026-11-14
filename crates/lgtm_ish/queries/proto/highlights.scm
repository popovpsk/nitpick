[
  "syntax"
  "edition"
  "package"
  "import"
  "public"
  "weak"
  "option"
  "message"
  "enum"
  "oneof"
  "map"
  "extend"
  "extensions"
  "reserved"
  "to"
  "max"
  "service"
  "rpc"
  "returns"
  "stream"
  "repeated"
  "optional"
  "required"
] @keyword

[
  (type)
  (key_type)
  (message_name)
  (enum_name)
  (service_name)
] @type

(rpc_name) @function

(field (identifier) @property)
(map_field (identifier) @property)
(oneof_field (identifier) @property)
(enum_field (identifier) @constant)

(string) @string
(escape_sequence) @string.escape

[
  (int_lit)
  (float_lit)
] @number

[
  (true)
  (false)
] @boolean

(comment) @comment

[
  "("
  ")"
  "["
  "]"
  "{"
  "}"
  "<"
  ">"
] @punctuation.bracket

[
  ";"
  ","
  "."
] @punctuation.delimiter

"=" @operator
