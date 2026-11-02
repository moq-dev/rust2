// Formatting as core::fmt does it: `Arguments` and its template, the `Formatter` and its
// padding, the debug builders and their pretty-printing, and the integers' impls.

package rt

import (
	"strconv"
	"unicode/utf8"
)

const (
	flagSignPlus      = 1 << 21
	flagSignMinus     = 1 << 22
	flagAlternate     = 1 << 23
	flagZeroPad       = 1 << 24
	flagDebugLowerHex = 1 << 25
	flagDebugUpperHex = 1 << 26
	flagWidth         = 1 << 27
	flagPrecision     = 1 << 28
	alignBits         = 3 << 29
	alignLeft         = 0 << 29
	alignRight        = 1 << 29
	alignCenter       = 2 << 29
	alignUnknown      = 3 << 29
	// The flags of `FormattingOptions::new()`: a space for fill, and no alignment.
	defaultFlags = ' ' | alignUnknown
)

type fmtResult = Result[struct{}, fmtError]

var fmtOk = fmtResult{}
var fmtErr = fmtResult{tag: 1}

// sink is what a Formatter writes to: `fmt::Write`.
type sink interface{ writeStr(s []uint8) fmtResult }

// byteSink appends to bytes, as a `String` does.
type byteSink struct{ buf *[]uint8 }

func (b byteSink) writeStr(s []uint8) fmtResult {
	*b.buf = append(*b.buf, s...)
	return fmtOk
}

// padAdapter indents what it writes by four spaces after each newline, as the debug
// builders' pretty-printing does.
type padAdapter struct {
	inner     sink
	onNewline *bool
}

func (p padAdapter) writeStr(s []uint8) fmtResult {
	for len(s) > 0 {
		line := s
		for i, c := range s {
			if c == '\n' {
				line = s[:i+1]
				break
			}
		}
		if *p.onNewline {
			if r := p.inner.writeStr([]uint8("    ")); r.tag != 0 {
				return r
			}
		}
		*p.onNewline = line[len(line)-1] == '\n'
		if r := p.inner.writeStr(line); r.tag != 0 {
			return r
		}
		s = s[len(line):]
	}
	return fmtOk
}

// Formatter is Rust's `fmt::Formatter`.
type Formatter struct {
	out       sink
	flags     uint32
	width     uint16
	precision uint16
}

func newFormatter(out sink) *Formatter { return &Formatter{out: out, flags: defaultFlags} }

// Argument is `fmt::rt::Argument`: a value's formatting function, or a count.
type Argument struct {
	fmt   func(*Formatter) fmtResult
	count uint16
}

func argumentOf(f func(*Formatter) fmtResult) Argument { return Argument{fmt: f} }

func argumentCount(n uint) Argument {
	if n > 1<<16-1 {
		panic(RustPanic{"Formatting argument out of range"})
	}
	return Argument{count: uint16(n)}
}

// Arguments is `fmt::Arguments`: its template, as core encodes it, and its arguments; or
// a string literal alone.
type Arguments struct {
	template []uint8
	args     []Argument
	str      []uint8
	isStr    bool
}

func argumentsStr(s []uint8) Arguments { return Arguments{str: s, isStr: true} }

// fmtDyn is `&dyn Display` or `&dyn Debug`: its `fmt`.
type fmtDyn func(*Formatter) fmtResult

func le16At(b []uint8) uint16 { return uint16(b[0]) | uint16(b[1])<<8 }

// write is `fmt::write`, reading the template as core encodes it.
func write(out sink, a Arguments) fmtResult {
	if a.isStr {
		return out.writeStr(a.str)
	}
	t := a.template
	argIndex := 0
	for {
		n := t[0]
		t = t[1:]
		switch {
		case n == 0:
			return fmtOk
		case n < 0x80:
			if r := out.writeStr(t[:n]); r.tag != 0 {
				return r
			}
			t = t[n:]
		case n == 0x80:
			l := int(le16At(t))
			if r := out.writeStr(t[2 : 2+l]); r.tag != 0 {
				return r
			}
			t = t[2+l:]
		default:
			f := &Formatter{out: out, flags: defaultFlags}
			if n&1 != 0 {
				f.flags = uint32(t[0]) | uint32(t[1])<<8 | uint32(t[2])<<16 | uint32(t[3])<<24
				t = t[4:]
			}
			if n&2 != 0 {
				f.width = le16At(t)
				t = t[2:]
			}
			if n&4 != 0 {
				f.precision = le16At(t)
				t = t[2:]
			}
			if n&8 != 0 {
				argIndex = int(le16At(t))
				t = t[2:]
			}
			if n&16 != 0 {
				f.width = a.args[f.width].count
			}
			if n&32 != 0 {
				f.precision = a.args[f.precision].count
			}
			if r := a.args[argIndex].fmt(f); r.tag != 0 {
				return r
			}
			argIndex++
		}
	}
}

// format is `alloc::fmt::format`, which panics if a formatting implementation fails.
func format(a Arguments) []uint8 {
	var buf []uint8
	if write(byteSink{&buf}, a).tag != 0 {
		panic(RustPanic{"a formatting trait implementation returned an error when the underlying stream did not: Error"})
	}
	return buf
}

func displayBytes(f func(*Formatter) fmtResult) []uint8 {
	var buf []uint8
	if f(newFormatter(byteSink{&buf})).tag != 0 {
		panic(RustPanic{"a Display implementation returned an error unexpectedly: Error"})
	}
	return buf
}

// rustPanicFmt is a panic's message, which keeps what was written before a formatting
// implementation failed, as std's does.
func rustPanicFmt(a Arguments) RustPanic {
	var buf []uint8
	write(byteSink{&buf}, a)
	return RustPanic{string(buf)}
}

func (f *Formatter) writeStr(s []uint8) fmtResult { return f.out.writeStr(s) }

func (f *Formatter) writeFmt(a Arguments) fmtResult { return write(f.out, a) }

func (f *Formatter) writeChar(c rune) fmtResult {
	return f.out.writeStr(utf8.AppendRune(nil, c))
}

func (f *Formatter) alternate() bool { return f.flags&flagAlternate != 0 }

func (f *Formatter) fill() rune { return rune(f.flags & (1<<21 - 1)) }

// padding writes the fill before a value `padding` chars short of the width, and gives
// the count to write after it, by the alignment or else `def`.
func (f *Formatter) padding(padding uint16, def uint32) (uint16, fmtResult) {
	align := f.flags & alignBits
	if align == alignUnknown {
		align = def
	}
	var left uint16
	switch align {
	case alignRight:
		left = padding
	case alignCenter:
		left = padding / 2
	}
	fill := utf8.AppendRune(nil, f.fill())
	for i := uint16(0); i < left; i++ {
		if r := f.out.writeStr(fill); r.tag != 0 {
			return 0, r
		}
	}
	return padding - left, fmtOk
}

func (f *Formatter) postPadding(n uint16) fmtResult {
	fill := utf8.AppendRune(nil, f.fill())
	for i := uint16(0); i < n; i++ {
		if r := f.out.writeStr(fill); r.tag != 0 {
			return r
		}
	}
	return fmtOk
}

// pad is `Formatter::pad`: a string truncated to the precision and padded to the width.
func (f *Formatter) pad(s []uint8) fmtResult {
	if f.flags&(flagWidth|flagPrecision) == 0 {
		return f.out.writeStr(s)
	}
	count := utf8.RuneCount(s)
	if f.flags&flagPrecision != 0 {
		end, n := 0, 0
		for end < len(s) && n < int(f.precision) {
			_, size := utf8.DecodeRune(s[end:])
			end += size
			n++
		}
		s, count = s[:end], n
	}
	if count < int(f.width) {
		post, r := f.padding(f.width-uint16(count), alignLeft)
		if r.tag != 0 {
			return r
		}
		if r := f.out.writeStr(s); r.tag != 0 {
			return r
		}
		return f.postPadding(post)
	}
	return f.out.writeStr(s)
}

// padIntegral is `Formatter::pad_integral`: digits with their sign and prefix, padded.
func (f *Formatter) padIntegral(nonneg bool, prefix []uint8, digits []uint8) fmtResult {
	width := len(digits)
	var sign []uint8
	if !nonneg {
		sign = []uint8("-")
		width++
	} else if f.flags&flagSignPlus != 0 {
		sign = []uint8("+")
		width++
	}
	if f.alternate() {
		width += utf8.RuneCount(prefix)
	} else {
		prefix = nil
	}
	writePrefix := func() fmtResult {
		if r := f.out.writeStr(sign); r.tag != 0 {
			return r
		}
		return f.out.writeStr(prefix)
	}
	min := int(f.width)
	switch {
	case width >= min:
		if r := writePrefix(); r.tag != 0 {
			return r
		}
		return f.out.writeStr(digits)
	case f.flags&flagZeroPad != 0:
		old := f.flags
		f.flags = f.flags&^(alignBits|(1<<21-1)) | '0' | alignRight
		if r := writePrefix(); r.tag != 0 {
			return r
		}
		post, r := f.padding(uint16(min-width), alignRight)
		if r.tag != 0 {
			return r
		}
		if r := f.out.writeStr(digits); r.tag != 0 {
			return r
		}
		r = f.postPadding(post)
		f.flags = old
		return r
	default:
		post, r := f.padding(uint16(min-width), alignRight)
		if r.tag != 0 {
			return r
		}
		if r := writePrefix(); r.tag != 0 {
			return r
		}
		if r := f.out.writeStr(digits); r.tag != 0 {
			return r
		}
		return f.postPadding(post)
	}
}

func fmtUint(v uint64, f *Formatter) fmtResult {
	return f.padIntegral(true, nil, strconv.AppendUint(nil, v, 10))
}

func fmtSint(v int64, f *Formatter) fmtResult {
	mag := uint64(v)
	if v < 0 {
		mag = -mag
	}
	return f.padIntegral(v >= 0, nil, strconv.AppendUint(nil, mag, 10))
}

// fmtHex is `LowerHex` or `UpperHex` of an integer `bits` wide: its two's complement.
func fmtHex(v uint64, bits uint, upper bool, f *Formatter) fmtResult {
	v &= ^uint64(0) >> (64 - bits)
	digits := strconv.AppendUint(nil, v, 16)
	if upper {
		for i, c := range digits {
			if c >= 'a' {
				digits[i] = c - 'a' + 'A'
			}
		}
	}
	return f.padIntegral(true, []uint8("0x"), digits)
}

func fmtUintDebug(v uint64, bits uint, f *Formatter) fmtResult {
	switch {
	case f.flags&flagDebugLowerHex != 0:
		return fmtHex(v, bits, false, f)
	case f.flags&flagDebugUpperHex != 0:
		return fmtHex(v, bits, true, f)
	}
	return fmtUint(v, f)
}

func fmtSintDebug(v int64, bits uint, f *Formatter) fmtResult {
	switch {
	case f.flags&flagDebugLowerHex != 0:
		return fmtHex(uint64(v), bits, false, f)
	case f.flags&flagDebugUpperHex != 0:
		return fmtHex(uint64(v), bits, true, f)
	}
	return fmtSint(v, f)
}

// DebugBuilder is `DebugStruct`, `DebugTuple`, and `DebugList`.
type DebugBuilder struct {
	f         *Formatter
	result    fmtResult
	fields    int
	emptyName bool
	tuple     bool
	list      bool
}

// wrapped is a formatter writing through a pad adapter, with f's options.
func (f *Formatter) wrapped(onNewline *bool) *Formatter {
	return &Formatter{out: padAdapter{f.out, onNewline}, flags: f.flags, width: f.width, precision: f.precision}
}

func (f *Formatter) debugStruct(name []uint8) DebugBuilder {
	return DebugBuilder{f: f, result: f.writeStr(name)}
}

func (f *Formatter) debugTuple(name []uint8) DebugBuilder {
	return DebugBuilder{f: f, result: f.writeStr(name), emptyName: len(name) == 0, tuple: true}
}

func (f *Formatter) debugList() DebugBuilder {
	return DebugBuilder{f: f, result: f.writeStr([]uint8("[")), list: true}
}

// field is `DebugStruct::field`, or `DebugTuple::field` without a name.
func (b *DebugBuilder) field(name []uint8, value fmtDyn) *DebugBuilder {
	if b.result.tag != 0 {
		return b
	}
	f := b.f
	if b.tuple {
		b.result = b.tupleField(value)
	} else if f.alternate() {
		if b.fields == 0 {
			if b.result = f.writeStr([]uint8(" {\n")); b.result.tag != 0 {
				return b
			}
		}
		onNewline := true
		w := f.wrapped(&onNewline)
		b.result = then(w.writeStr(name), func() fmtResult { return w.writeStr([]uint8(": ")) },
			func() fmtResult { return value(w) }, func() fmtResult { return w.writeStr([]uint8(",\n")) })
	} else {
		prefix := " { "
		if b.fields > 0 {
			prefix = ", "
		}
		b.result = then(f.writeStr([]uint8(prefix)), func() fmtResult { return f.writeStr(name) },
			func() fmtResult { return f.writeStr([]uint8(": ")) }, func() fmtResult { return value(f) })
	}
	b.fields++
	return b
}

func (b *DebugBuilder) tupleField(value fmtDyn) fmtResult {
	f := b.f
	if f.alternate() {
		r := fmtOk
		if b.fields == 0 {
			r = f.writeStr([]uint8("(\n"))
		}
		onNewline := true
		w := f.wrapped(&onNewline)
		return then(r, func() fmtResult { return value(w) }, func() fmtResult { return w.writeStr([]uint8(",\n")) })
	}
	prefix := "("
	if b.fields > 0 {
		prefix = ", "
	}
	return then(f.writeStr([]uint8(prefix)), func() fmtResult { return value(f) })
}

// entry is `DebugList::entry`.
func (b *DebugBuilder) entry(value fmtDyn) *DebugBuilder {
	if b.result.tag != 0 {
		return b
	}
	f := b.f
	if f.alternate() {
		r := fmtOk
		if b.fields == 0 {
			r = f.writeStr([]uint8("\n"))
		}
		onNewline := true
		w := f.wrapped(&onNewline)
		b.result = then(r, func() fmtResult { return value(w) }, func() fmtResult { return w.writeStr([]uint8(",\n")) })
	} else {
		r := fmtOk
		if b.fields > 0 {
			r = f.writeStr([]uint8(", "))
		}
		b.result = then(r, func() fmtResult { return value(f) })
	}
	b.fields++
	return b
}

func (b *DebugBuilder) finish() fmtResult {
	if b.result.tag != 0 {
		return b.result
	}
	f := b.f
	switch {
	case b.list:
		return f.writeStr([]uint8("]"))
	case b.fields == 0:
		return fmtOk
	case b.tuple:
		if b.fields == 1 && b.emptyName && !f.alternate() {
			if r := f.writeStr([]uint8(",")); r.tag != 0 {
				return r
			}
		}
		return f.writeStr([]uint8(")"))
	case f.alternate():
		return f.writeStr([]uint8("}"))
	}
	return f.writeStr([]uint8(" }"))
}

func (b *DebugBuilder) finishNonExhaustive() fmtResult {
	if b.result.tag != 0 {
		return b.result
	}
	f := b.f
	if f.alternate() {
		r := fmtOk
		if b.fields == 0 {
			r = f.writeStr([]uint8(" {\n"))
		}
		onNewline := true
		w := f.wrapped(&onNewline)
		return then(r, func() fmtResult { return w.writeStr([]uint8("..\n")) }, func() fmtResult { return f.writeStr([]uint8("}")) })
	}
	if b.fields > 0 {
		return f.writeStr([]uint8(", .. }"))
	}
	return f.writeStr([]uint8(" { .. }"))
}

func (f *Formatter) debugTupleFields(name []uint8, values ...fmtDyn) fmtResult {
	b := f.debugTuple(name)
	for _, v := range values {
		b.field(nil, v)
	}
	return b.finish()
}

func (f *Formatter) debugStructFields(name []uint8, names [][]uint8, values []fmtDyn) fmtResult {
	b := f.debugStruct(name)
	for i := range names {
		b.field(names[i], values[i])
	}
	return b.finish()
}

// then runs each step while the last succeeded, as `?` does.
func then(r fmtResult, steps ...func() fmtResult) fmtResult {
	for _, step := range steps {
		if r.tag != 0 {
			return r
		}
		r = step()
	}
	return r
}
