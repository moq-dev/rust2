package bench

// The codec's C ABI, as rust2-ffi exports it, which `just go bench` builds as a static
// library and points CGO_CFLAGS and CGO_LDFLAGS at.

/*
#cgo LDFLAGS: -lm -ldl -lpthread
#include "codec.h"
*/
import "C"

import "unsafe"

// native runs the workload through cgo: the input passed as a view, and the output
// copied into Go memory and freed, as a Go user of the export does.
func native(input []uint8) []uint8 {
	view := C.codec_buf{ptr: (*C.uint8_t)(unsafe.SliceData(input)), len: C.size_t(len(input))}
	out := C.codec_bench_run(view)
	defer C.codec_buf_free(out)
	return C.GoBytes(unsafe.Pointer(out.ptr), C.int(out.len))
}

func contract() bool { return C.codec_contract() == C.CODEC_CONTRACT }
