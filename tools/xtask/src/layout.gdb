# GDB's type API is available only through its embedded Python interpreter.
# This observer never executes the subject. Rust owns acceptance and comparisons.
python
import gdb, json
probe = ['rune::experimental_core::detail::Detail', 'rune::experimental_core::error::Error', 'rune::experimental_core::execute::Failure', 'core::option::Option<rune::experimental_core::error::Error>', 'core::result::Result<rune::experimental_core::value::Value, rune::experimental_core::error::Error>', 'core::result::Result<(), rune::experimental_core::error::Error>', 'core::result::Result<rune::experimental_core::value::Value, rune::experimental_core::execute::Failure>']
variants = ['rune::experimental_core::detail::Detail::NumericArgumentCoerce', 'rune::experimental_core::detail::Detail::Coerce', 'rune::experimental_core::detail::IntValue', 'rune::experimental_core::detail::IntTarget', 'core::option::Option<rune::experimental_core::detail::Detail>']
result = {'layout_probe': {}, 'layout_variants': {}}
for name in probe:
    t = gdb.lookup_type(name)
    result['layout_probe'][name] = dict(size=t.sizeof, align=t.alignof, fields=[dict(name=f.name, type=str(f.type), bitpos=f.bitpos, bitsize=f.bitsize, artificial=f.artificial) for f in t.fields()])
for name in variants:
    try:
        t = gdb.lookup_type(name)
    except gdb.error:
        result['layout_variants'][name] = None
        continue
    result['layout_variants'][name] = dict(size=t.sizeof, align=t.alignof, fields=[dict(name=f.name, type=str(f.type), bitpos=getattr(f, 'bitpos', None), artificial=f.artificial, size=None if f.type is None else f.type.sizeof) for f in t.fields()])
print('LAYOUT_JSON ' + json.dumps(result))
end
