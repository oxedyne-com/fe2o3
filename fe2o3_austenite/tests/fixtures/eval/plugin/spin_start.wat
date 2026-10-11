(module $spin_start
  (import "typst_env" "wasm_minimal_protocol_write_args_to_buffer" (func (param i32) (result )))
  (import "typst_env" "wasm_minimal_protocol_send_result_to_host" (func (param i32 i32) (result )))
  (memory (export "memory") 1)
  (func (export "spin") (param ) (result )
    loop
    br 0
    end
  )
  (func (export "f") (param ) (result i32)
    i32.const 0
  )
  (start $spin)
)
