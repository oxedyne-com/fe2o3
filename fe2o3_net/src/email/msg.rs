use crate::{
    charset::Charset,
    constant,
    media::{
        ContentTypeValue,
        MediaType,
    },
    smtp::msg::SmtpMessage,
};

use oxedyne_fe2o3_core::{
    prelude::*,
    count::ErrorWhen,
};

use std::{
    fmt,
};

use tokio::{
    io::{
        AsyncRead,
        //AsyncReadExt,
        AsyncBufRead,
        //AsyncBufReadExt,
        //AsyncWriteExt,
    },
};


#[derive(Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct EmailMessage {
    pub from:       String,
    pub to:         Vec<String>,
    pub subject:    String,
    pub body:       String,
    pub headers:    Vec<EmailHeader>,
}

impl fmt::Display for EmailMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "From: {}", self.from)?;
        writeln!(f, "To: {}", self.to.join(", "))?;
        writeln!(f, "Subject: {}", self.subject)?;
        for header in &self.headers {
            writeln!(f, "{}", header)?;
        }
        writeln!(f)?;
        write!(f, "{}", self.body)
    }
}

impl EmailMessage {

    pub async fn read<R: AsyncRead + AsyncBufRead + Unpin>(
        stream: &mut R,
    )
        -> Outcome<EmailMessage>
    {
        let mut email = Self::default();
        let mut in_headers = true;
        let mut header_line = String::new();
    
        let mut safety = ErrorWhen::new(constant::READ_LOOP_SAFETY_LIMIT);
        loop {
            res!(safety.inc());
            let mut line = Vec::new();
            let result = SmtpMessage::read_line(stream, &mut line).await;
            let byts_read = res!(result);
    
            if byts_read == 1 {
                if line[0] == b'.' {
                    break;
                }
            }
            if byts_read == 2 {
                if line[0] == b'\r' && line[1] == b'\n' {
                    in_headers = false;
                    continue;
                }
            }
    
            let line = String::from_utf8_lossy(&line);
            debug!("### line={}", line);
            let line = line.trim_end();
    
            if line.is_empty() && in_headers {
                in_headers = false;
                continue;
            }
    
            if in_headers {
                if line.starts_with(char::is_whitespace) { 
                    // Accumulate header lines.
                    header_line.push(' ');
                    header_line.push_str(line.trim_start());
                } else {
                    if !header_line.is_empty() {
                        match res!(EmailHeader::from_str(&header_line)) {
                            EmailHeader::From(value) => email.from = value,
                            EmailHeader::To(value) => email.to.push(value),
                            EmailHeader::Subject(value) => email.subject = value,
                            header => {
                                email.headers.push(header);
                            }
                        }
                    }
                    header_line = line.to_string();
                }
            } else {
                // Accumulate body lines.
                email.body.push_str(line);
                email.body.push('\n');
            }
        }
    
        Ok(email)
    }

}

/// RFC 5322 Internet Message Format https://datatracker.ietf.org/doc/html/rfc5322
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum EmailHeader {
    From(String),
    To(String),
    Cc(String),
    Bcc(String),
    Subject(String),
    Date(String),
    MessageId(String),
    InReplyTo(String),
    References(String),
    ResentFrom(String),
    ResentTo(String),
    ResentCc(String),
    ResentBcc(String),
    ResentDate(String),
    ResentMessageId(String),
    ReplyTo(String),
    Sender(String),
    ReturnPath(String),
    ContentType(ContentTypeValue),
    ContentTransferEncoding(String),
    ContentDisposition(ContentDisposition),
    ContentId(String),
    MimeVersion(String),
    XMailer(String),
    XPriority(String),
    XMsMailPriority(String),
    Importance(String),
    Received(String),
    Other(String, String),
}

impl Default for EmailHeader {
    fn default() -> Self {
        Self::Other(String::new(), String::new())
    }
}

impl fmt::Display for EmailHeader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EmailHeader::From(value) => write!(f, "From: {}", value),
            EmailHeader::To(value) => write!(f, "To: {}", value),
            EmailHeader::Cc(value) => write!(f, "Cc: {}", value),
            EmailHeader::Bcc(value) => write!(f, "Bcc: {}", value),
            EmailHeader::Subject(value) => write!(f, "Subject: {}", value),
            EmailHeader::Date(value) => write!(f, "Date: {}", value),
            EmailHeader::MessageId(value) => write!(f, "Message-ID: {}", value),
            EmailHeader::InReplyTo(value) => write!(f, "In-Reply-To: {}", value),
            EmailHeader::References(value) => write!(f, "References: {}", value),
            EmailHeader::ResentFrom(value) => write!(f, "Resent-From: {}", value),
            EmailHeader::ResentTo(value) => write!(f, "Resent-To: {}", value),
            EmailHeader::ResentCc(value) => write!(f, "Resent-Cc: {}", value),
            EmailHeader::ResentBcc(value) => write!(f, "Resent-Bcc: {}", value),
            EmailHeader::ResentDate(value) => write!(f, "Resent-Date: {}", value),
            EmailHeader::ResentMessageId(value) => write!(f, "Resent-Message-ID: {}", value),
            EmailHeader::ReplyTo(value) => write!(f, "Reply-To: {}", value),
            EmailHeader::Sender(value) => write!(f, "Sender: {}", value),
            EmailHeader::ReturnPath(value) => write!(f, "Return-Path: {}", value),
            EmailHeader::ContentType(content_type) => write!(f, "Content-Type: {}", content_type),
            EmailHeader::ContentTransferEncoding(value) => write!(f, "Content-Transfer-Encoding: {}", value),
            EmailHeader::ContentDisposition(disposition) => write!(f, "Content-Disposition: {}", disposition),
            EmailHeader::ContentId(value) => write!(f, "Content-ID: {}", value),
            EmailHeader::MimeVersion(value) => write!(f, "MIME-Version: {}", value),
            EmailHeader::XMailer(value) => write!(f, "X-Mailer: {}", value),
            EmailHeader::XPriority(value) => write!(f, "X-Priority: {}", value),
            EmailHeader::XMsMailPriority(value) => write!(f, "X-MS-Mail-Priority: {}", value),
            EmailHeader::Importance(value) => write!(f, "Importance: {}", value),
            EmailHeader::Received(value) => write!(f, "Received: {}", value),
            EmailHeader::Other(name, value) => write!(f, "{}: {}", name, value),
        }
    }
}

impl FromStr for EmailHeader {
    type Err = Error<ErrTag>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.splitn(2, ':').map(str::trim).collect();
        if parts.len() != 2 {
            return Err(err!("Invalid email header format: {}", s; Invalid, Input));
        }

        let name = parts[0].to_lowercase();
        let value = parts[1].to_string();

        match name.as_str() {
            "from" => Ok(EmailHeader::From(value)),
            "to" => Ok(EmailHeader::To(value)),
            "cc" => Ok(EmailHeader::Cc(value)),
            "bcc" => Ok(EmailHeader::Bcc(value)),
            "subject" => Ok(EmailHeader::Subject(value)),
            "date" => Ok(EmailHeader::Date(value)),
            "message-id" => Ok(EmailHeader::MessageId(value)),
            "in-reply-to" => Ok(EmailHeader::InReplyTo(value)),
            "references" => Ok(EmailHeader::References(value)),
            "resent-from" => Ok(EmailHeader::ResentFrom(value)),
            "resent-to" => Ok(EmailHeader::ResentTo(value)),
            "resent-cc" => Ok(EmailHeader::ResentCc(value)),
            "resent-bcc" => Ok(EmailHeader::ResentBcc(value)),
            "resent-date" => Ok(EmailHeader::ResentDate(value)),
            "resent-message-id" => Ok(EmailHeader::ResentMessageId(value)),
            "reply-to" => Ok(EmailHeader::ReplyTo(value)),
            "sender" => Ok(EmailHeader::Sender(value)),
            "return-path" => Ok(EmailHeader::ReturnPath(value)),
            "content-type" => {
                let content_type = res!(MediaType::from_str(value.as_str()));
                let content_type_value = match content_type {
                    MediaType::Text(ref _text) => {
                        ContentTypeValue::MediaType((content_type, Some(Charset::Utf_8)))
                    }
                    MediaType::Multipart(multipart) => {
                        ContentTypeValue::Multipart((multipart, String::new()))
                    }
                    _ => ContentTypeValue::MediaType((content_type, None)),
                };
                Ok(EmailHeader::ContentType(content_type_value))
            },
            "content-transfer-encoding" => Ok(EmailHeader::ContentTransferEncoding(value)),
            "content-disposition" => {
                let disposition = res!(ContentDisposition::from_str(&value));
                Ok(EmailHeader::ContentDisposition(disposition))
            },
            "content-id" => Ok(EmailHeader::ContentId(value)),
            "mime-version" => Ok(EmailHeader::MimeVersion(value)),
            "x-mailer" => Ok(EmailHeader::XMailer(value)),
            "x-priority" => Ok(EmailHeader::XPriority(value)),
            "x-ms-mail-priority" => Ok(EmailHeader::XMsMailPriority(value)),
            "importance" => Ok(EmailHeader::Importance(value)),
            "received" => Ok(EmailHeader::Received(value)),
            _ => Ok(EmailHeader::Other(name, value)),
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ContentDisposition {
    Inline,
    Attachment(Option<String>),
}

impl fmt::Display for ContentDisposition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContentDisposition::Inline => write!(f, "inline"),
            ContentDisposition::Attachment(Some(filename)) => {
                write!(f, "attachment; filename={}", filename)
            }
            ContentDisposition::Attachment(None) => write!(f, "attachment"),
        }
    }
}

impl FromStr for ContentDisposition {
    type Err = Error<ErrTag>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split(';').map(str::trim).collect();
        match parts[0].to_lowercase().as_str() {
            "inline" => Ok(ContentDisposition::Inline),
            "attachment" => {
                if parts.len() > 1 {
                    let filename = parts[1].trim_start_matches("filename=").to_string();
                    Ok(ContentDisposition::Attachment(Some(filename)))
                } else {
                    Ok(ContentDisposition::Attachment(None))
                }
            }
            _ => Err(err!(
                "Invalid Content-Disposition value: {}", s;
            Invalid, Input)),
        }
    }
}
