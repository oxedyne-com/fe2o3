use oxedize_fe2o3_net::{
    //conc::AsyncReadIterator,
    dns::Fqdn,
    //email::msg::{
    //    EmailHeader,
    //    EmailMessage,
    //},
    smtp::{
        cmd::SmtpCommand,
        //msg::SmtpMessageReader,
    },
};

use oxedize_fe2o3_core::{
    prelude::*,
    test::test_it,
};
//use oxedize_fe2o3_text::string::Stringer;
//
//use std::{
//    pin::Pin,
//};


pub fn test_smtp(filter: &'static str) -> Outcome<()> {

// S: 220 serverb.example.com ESMTP Postfix
// C: EHLO servera.example.org
// S: 250-serverb.example.com
// S: 250-PIPELINING
// S: 250-SIZE 10240000
// S: 250-VRFY
// S: 250-ETRN
// S: 250-STARTTLS
// S: 250-ENHANCEDSTATUSCODES
// S: 250-8BITMIME
// S: 250 DSN
// C: MAIL FROM:<sender@example.org>
// S: 250 2.1.0 Ok
// C: RCPT TO:<recipient@example.com>
// S: 250 2.1.5 Ok
// C: DATA
// S: 354 End data with <CR><LF>.<CR><LF>
// C: Received: from servera.example.org (servera.example.org [192.168.0.1])
// C:         by serverb.example.com (Postfix) with ESMTP id 12345678
// C:         for <recipient@example.com>; Fri, 11 Jun 2023 09:30:00 -0500 (EST)
// C: From: Sender <sender@example.org>
// C: To: Recipient <recipient@example.com>
// C: Subject: Test message
// C: Date: Fri, 11 Jun 2023 09:30:00 -0500
// C: Message-ID: <12345.67890@servera.example.org>
// C:
// C: This is a test message sent from Server A to Server B.
// C:
// C: Best regards,
// C: Sender
// C: .
// S: 250 2.0.0 Ok: queued as ABCDEF123456
// C: QUIT
// S: 221 2.0.0 Bye

    res!(test_it(filter, &["Smtp commands 000", "all", "smtp"], || {
        let test = [
            ("HELO client.example.com", SmtpCommand::Helo(res!(Fqdn::new("client.example.com")))),
            ("EHLO client.example.com", SmtpCommand::Ehlo(res!(Fqdn::new("client.example.com")))),
            ("MAIL FROM:<sender@example.com>", SmtpCommand::MailFrom(fmt!("sender@example.com"))),
            ("RCPT TO:<sender@example.com>", SmtpCommand::RcptTo(fmt!("sender@example.com"))),
        ];
        for (wire, expected) in test {
            let cmd = res!(SmtpCommand::from_str(wire));
            req!(expected, cmd, "(L: expected, R: actual)");
        }
        Ok(())
    }));

    Ok(())
}
