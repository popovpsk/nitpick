[
  "query"
  "mutation"
  "subscription"
  "fragment"
  "on"
  "type"
  "interface"
  "union"
  "enum"
  "input"
  "scalar"
  "schema"
  "extend"
  "directive"
  "implements"
  "repeatable"
] @keyword

(named_type (name) @type)
(object_type_definition (name) @type)
(interface_type_definition (name) @type)
(union_type_definition (name) @type)
(enum_type_definition (name) @type)
(input_object_type_definition (name) @type)
(scalar_type_definition (name) @type)

(operation_definition (name) @function)
(fragment_name (name) @function)

(field_definition (name) @property)
(field (name) @property)
(object_field (name) @property)
(alias (name) @label)

(input_value_definition (name) @variable)
(argument (name) @variable)
(variable) @variable.special

(directive "@" @attribute)
(directive (name) @attribute)
(directive_definition "@" @attribute)
(directive_definition (name) @attribute)
(directive_location) @constant

(enum_value) @constant
(null_value) @constant
(boolean_value) @boolean
(int_value) @number
(float_value) @number
(string_value) @string
(comment) @comment

[
  "("
  ")"
  "["
  "]"
  "{"
  "}"
] @punctuation.bracket

":" @punctuation.delimiter

[
  "="
  "!"
  "|"
  "&"
  "..."
] @operator
