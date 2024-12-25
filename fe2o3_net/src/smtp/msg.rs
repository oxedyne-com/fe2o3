use crate::{
    conc::AsyncReadIterator, 
    constant,
    email::msg::EmailMessage,
    smtp::cmd::SmtpCommand,
};

use oxedize_fe2o3_core::{
    prelude::*,
    count::ErrorWhen,
};

use std::{
    future::Future,
    pin::Pin,
};

use tokio::{
    io::{
        AsyncRead,
        //AsyncReadExt,
        AsyncBufRead,
        AsyncBufReadExt,
        //AsyncWriteExt,
    },
};


new_type!(SmtpMessage, Vec<SmtpCommand>, Debug, Default);

impl SmtpMessage {

    pub async fn read<R: AsyncRead + AsyncBufRead + Unpin>(
        stream: &mut R,
        _buffer: &mut Vec<u8>,
    )
        -> Outcome<Option<Self>>
    {
        let mut message = SmtpMessage::default();

        let mut safety = ErrorWhen::new(constant::READ_LOOP_SAFETY_LIMIT);
        loop {
            res!(safety.inc());
            let mut line = Vec::new();
            let result = Self::read_line(stream, &mut line).await;
            let byts_read = res!(result);

            if byts_read == 0 {
                if message.is_empty() {
                    return Ok(None);
                } else {
                    break;
                }
            }

            let line = String::from_utf8_lossy(&line);
            let line = line.trim_end();

            if line.is_empty() {
                continue;
            }

            // Handle multiline responses.
            if line.starts_with('-') {
                // Remove the '-' prefix and append the line to the previous command.
                let mut last_command = match message.pop() {
                    Some(command) => command,
                    None => return Err(err!(errmsg!(
                        "Unexpected end of message",
                    ), Unexpected, Input, Missing)),
                };
                let multiline_response = line[1..].trim();
                match last_command {
                    SmtpCommand::Response(ref _code, ref mut msg) => {
                        msg.push('\n');
                        msg.push_str(multiline_response);
                    }
                    _ => return Err(err!(errmsg!(
                        "Unexpected multiline response: {}", line,
                    ), Invalid, Input)),
                }
                message.push(last_command);
                continue;
            }

            // Parse the SMTP command
            match SmtpCommand::from_str(line) {
                Ok(command) => match command {
                    SmtpCommand::Data => {
                        let result = EmailMessage::read(stream).await;
                        let email = res!(result);
                        message.push(SmtpCommand::Email(email));
                    }
                    SmtpCommand::Quit => {
                        message.push(SmtpCommand::Quit);
                        break;
                    }
                    _ => {
                        message.push(command);
                    }
                },
                Err(e) => return Err(err!(e, errmsg!(
                    "Invalid SMTP command: {}", line,
                ), Invalid, Input)),
            }
        }

        Ok(Some(message))
    }

    pub async fn read_line<R: AsyncRead + AsyncBufRead + Unpin>(
        stream: &mut R,
        bfr:    &mut Vec<u8>,
    )
        -> Outcome<usize>
    {
        let mut tmp_bfr = Vec::new();
        let mut total_byts_read = 0;
    
        let mut safety = ErrorWhen::new(constant::READ_LOOP_SAFETY_LIMIT);
        loop {
            debug!("safety={:?}", safety);
            res!(safety.inc());
            debug!("safety={:?}", safety);
            let result = stream.read_until(b'\n', &mut tmp_bfr).await;
            let byts_read = res!(result, IO, Network, Read);
    
            if byts_read == 0 {
                break;
            }
    
            // Ignore fragment consisting only of '\n'.
            if byts_read > 1 {
                if tmp_bfr[byts_read - 2] == b'\r' {
                    // We've found the \r\n to end the line, remove it.
                    if byts_read > 2 {
                        let byts = &tmp_bfr[..byts_read - 2];
                        bfr.extend_from_slice(byts);
                        total_byts_read += byts_read - 2;
                    }
                    break
                } else {
                    // We only found an \n ending, continue accumulating.
                    let byts = &tmp_bfr[..byts_read];
                    bfr.extend_from_slice(byts);
                    total_byts_read += byts_read;
                }
            }
        }
        Ok(total_byts_read)
    }
    
}

pub struct SmtpMessageReader<
    'a,
    R: AsyncRead + AsyncBufRead + Unpin + Send
> {
    stream: Pin<&'a mut R>,
    buffer: Vec<u8>,
}

impl<
    'a,
    R: AsyncRead + AsyncBufRead + Unpin + Send
>
    SmtpMessageReader<'a, R>
{
    pub fn new(stream: Pin<&'a mut R>) -> Self {
        Self {
            stream,
            buffer: Vec::new(),
        }
    }
}

impl<
    'a,
    R: AsyncRead + AsyncBufRead + Unpin + Send
>
    AsyncReadIterator for SmtpMessageReader<'a, R>
{
    type Item = Outcome<SmtpMessage>;

    fn next<'b>(&'b mut self) -> Pin<Box<dyn Future<Output = Option<Self::Item>> + Send + 'b>> {
        let mut stream = self.stream.as_mut();
        let buffer = &mut self.buffer;

        Box::pin(async move {
            let result = SmtpMessage::read::<_>(
                &mut stream.as_mut(),
                buffer,
            )
            .await;

            match result {
                Ok(Some(message))   => Some(Ok(message)),
                Ok(None)            => None,
                Err(e)              => Some(Err(e)),
            }
        })
    }
}

//#[derive(Debug)]
//pub struct SmtpSession {
//    pub client_ip:          IpAddr,
//    pub client_hostname:    Fqdn,
//    pub messages:           Vec<SmtpCommand>,
//}
