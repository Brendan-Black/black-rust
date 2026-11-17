# Rust: seams

A seam is a trait. Take it as a generic (`fn new(gateway: G) where G: HttpGateway`, or `impl HttpGateway`) when the
implementation is fixed at compile time, which is nearly always; `Box<dyn HttpGateway>` when it's chosen at run time
or the generic parameter would spread through every type above it. Both let a pseudo stand in.
