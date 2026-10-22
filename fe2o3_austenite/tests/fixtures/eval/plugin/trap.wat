(module $trap
  (import "typst_env" "wasm_minimal_protocol_write_args_to_buffer" (func (param i32) (result )))
  (import "typst_env" "wasm_minimal_protocol_send_result_to_host" (func (param i32 i32) (result )))
  (memory (export "memory") 1)
  (func (export "boom") (param ) (result i32)
    unreachable
  )
)
