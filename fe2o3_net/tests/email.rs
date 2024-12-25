use oxedize_fe2o3_net::{
    dns::Fqdn,
    email::{
        file::{
            MboxEmailIterator,
        },
        msg::{
            EmailHeader,
            EmailMessage,
        },
    },
    smtp::{
        cmd::SmtpCommand,
    },
};

use oxedize_fe2o3_core::{
    prelude::*,
    test::test_it,
};
use oxedize_fe2o3_text::string::Stringer;

use std::{
    path::Path,
    pin::Pin,
};


pub fn test_email(filter: &'static str) -> Outcome<()> {

    res!(test_it(filter, &["Email message 000", "all", "email"], || {

        // Note: Every line ends with an implicit \n character.
        let wire = "Received: from servera.example.org (servera.example.org [192.168.0.1])\r
 by serverb.example.com (Postfix) with ESMTP id 12345678\r
 for <recipient@example.com>; Fri, 11 Jun 2023 09:30:00 -0500 (EST)\r
From: Sender <sender@example.org>\r
To: Recipient <recipient@example.com>\r
Subject: Test message\r
Date: Fri, 11 Jun 2023 09:30:00 -0500\r
Message-ID: <12345.67890@servera.example.org>\r
\r
This is a test message sent from Server A to Server B.\r
\r
Best regards,\r
Sender\r
.\r\n";

        let byts = wire.as_bytes();
        let mut stream = std::io::Cursor::new(byts);
        let rt = res!(tokio::runtime::Runtime::new());

        let result = rt.block_on(async {
            EmailMessage::read(&mut Pin::new(&mut stream)).await
        });

        let email = res!(result);
        debug!("{:?}", email);
        let expected = EmailMessage {
            from:       fmt!("Sender <sender@example.org>"),
            to:         vec![fmt!("Recipient <recipient@example.com>")],
            subject:    fmt!("Test message"),
            body:       fmt!("This is a test message sent from Server A to Server B.\n\nBest regards,\nSender\n"),
            headers:    vec![
                EmailHeader::Received(fmt!("from servera.example.org (servera.example.org [192.168.0.1]) by serverb.example.com (Postfix) with ESMTP id 12345678 for <recipient@example.com>; Fri, 11 Jun 2023 09:30:00 -0500 (EST)")),
                EmailHeader::Date(fmt!("Fri, 11 Jun 2023 09:30:00 -0500")),
            ],
        };
        for line in Stringer::new(fmt!("{:?}", email)).to_lines("  ") {
            test!("{}", line);
        }
        req!(expected, email, "(L: expected, R: actual)");
        Ok(())
    }));

    res!(test_it(filter, &["Mbox reader 000", "all", "email", "mbox"], || {

        let rt = res!(tokio::runtime::Runtime::new());

        let home = res!(std::env::var("HOME"));
        let path = Path::new(home).join("tmp/Inbox");
        let result = rt.block_on(async {
            MboxEmailIterator::new(path).await
        });

        let mut mbox_iter = res!(result);
        match mbox_iter.next() {
            Some(Ok(email)) => {
                test!("{:?}", email);
            }
            Some(Err(e)) => {
                return Err(err!(e, errmsg!(
                    "While reading first email from mbox file.",
                ), Test, IO, File, Read)); 
            }
            None => {
                return Err(err!(errmsg!(
                    "Expected to read an email from the mbox file.",
                ), Test, Missing, Data)); 
            }
        }
        Ok(())
    }));

    Ok(())
}
