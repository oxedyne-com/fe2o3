use oxedyne_fe2o3_net::{
    conc::AsyncReadIterator,
    constant,
    http::msg::HttpMessageReader,
};

use oxedyne_fe2o3_core::{
    prelude::*,
};

use std::{
    pin::Pin,
};


pub fn test_http(filter: &'static str) -> Outcome<()> {

    match filter {
        "all" | "request" => {
            let header = "POST /adduser HTTP/1.1\r\n\
Host: my.domain.com\r\n\
User-Agent: Mozilla/5.0 (X11; Ubuntu; Linux x86_64; rv:121.0) Gecko/20100101 Firefox/121.0\r\n\
Accept: text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8\r\n\
Accept-Language: en-US,en;q=0.5\r\n\
Accept-Encoding: gzip, deflate, br\r\n\
Content-Type: application/x-www-form-urlencoded\r\n\
Content-Length: 25\r\n\
Origin: null\r\n\
Connection: keep-alive\r\n\
Cookie: _ga_PYS2NF62KB=GS1.1.1703943898.1.1.1703943949.0.0.0; _ga=GA1.1.1548945696.1703943899\r\n\
Upgrade-Insecure-Requests: 1\r\n\
Sec-Fetch-Dest: document\r\n\
Sec-Fetch-Mode: navigate\r\n\
Sec-Fetch-Site: cross-site\r\n\
Sec-Fetch-User: ?1\r\n\r\n";
            let body = "username=jane&password=doe";
            let wire = fmt!("{}{}", header, body);
            let byts = wire.as_bytes();
            let mut stream = std::io::Cursor::new(byts);
            let rt = res!(tokio::runtime::Runtime::new());

            let mut reader: HttpMessageReader<
                '_,
                { constant::HTTP_DEFAULT_HEADER_CHUNK_SIZE },
                { constant::HTTP_DEFAULT_BODY_CHUNK_SIZE },
                _,
            > = HttpMessageReader::new(Pin::new(&mut stream));

            let result = rt.block_on(async {
                reader.next().await
            });

            if let Some(result) = result {
                let http_msg = res!(result);
                let body_str = http_msg.body_as_string();
                req!(&body_str, body);
                test!("Successfully parsed HTTP request.");
            } else {
                return Err(err!(
                    "Message reading should not have returned None.";
                Missing, Test));
            }
        },
        _ => (),
    }

    Ok(())
}
