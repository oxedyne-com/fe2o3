(module $grow
  (import "typst_env" "wasm_minimal_protocol_write_args_to_buffer" (func (param i32) (result )))
  (import "typst_env" "wasm_minimal_protocol_send_result_to_host" (func (param i32 i32) (result )))
  (memory (export "memory") 1)
  (func (export "grow") (param ) (result i32)
    loop
    i32.const 1
    memory.grow
    i32.const -1
    i32.ne
    br_if 0
    end
    unreachable
  )
)
