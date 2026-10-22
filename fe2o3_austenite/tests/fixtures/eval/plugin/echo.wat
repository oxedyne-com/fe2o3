(module $echo
  (import "typst_env" "wasm_minimal_protocol_write_args_to_buffer" (func (param i32) (result )))
  (import "typst_env" "wasm_minimal_protocol_send_result_to_host" (func (param i32 i32) (result )))
  (memory (export "memory") 1)
  (func (export "echo") (param i32) (result i32)
    i32.const 0
    call 0
    i32.const 0
    local.get 0
    call 1
    i32.const 0
  )
  (func (export "two") (param i32 i32) (result i32)
    i32.const 0
    call 0
    i32.const 0
    local.get 0
    local.get 1
    i32.add
    call 1
    i32.const 0
  )
  (func (export "none") (param ) (result i32)
    i32.const 0
    call 0
    i32.const 0
    i32.const 0
    call 1
    i32.const 0
  )
)
