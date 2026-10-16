(module $counter
  (import "typst_env" "wasm_minimal_protocol_write_args_to_buffer" (func (param i32) (result )))
  (import "typst_env" "wasm_minimal_protocol_send_result_to_host" (func (param i32 i32) (result )))
  (memory (export "memory") 1)
  (func (export "inc") (param ) (result i32)
    i32.const 0
    i32.const 0
    i32.load8_u offset=0
    i32.const 1
    i32.add
    i32.store8 offset=0
    i32.const 0
    i32.const 1
    call 1
    i32.const 0
  )
  (func (export "get") (param ) (result i32)
    i32.const 0
    i32.const 1
    call 1
    i32.const 0
  )
)
