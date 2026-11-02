module bench

go 1.24

require codec v0.0.0

// The translation, which `just go bench` writes.
replace codec => ../target/bench/codec
