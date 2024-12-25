use oxedize_fe2o3_core::prelude::*;

use std::{
    fmt,
};


#[derive(Clone, Debug, PartialEq)]
pub enum CompletionState {
    PositiveCompletion,
    PositiveIntermediate,
    TransientNegativeCompletion,
    PermanentNegativeCompletion,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SmtpResponseCode {
    NonStandardSuccess,
    SystemStatusReply,
    HelpMessage,
    ServiceReady,
    ServiceClosingTransmissionChannel,
    AuthenticationSuccessful,
    RequestedMailActionOkayCompleted,
    UserNotLocalWillForward,
    CannotVerifyUserButWillAttemptDelivery,
    AuthInputData,
    StartMailInput,
    ServiceNotAvailableClosingTransmissionChannel,
    PasswordTransitionNeeded,
    MailboxUnavailable,
    LocalErrorInProcessing,
    InsufficientSystemStorage,
    UnableToAccommodateParameters,
    CommandUnrecognized,
    SyntaxErrorInParameters,
    CommandNotImplemented,
    BadSequenceOfCommands,
    CommandParameterNotImplemented,
    DomainDoesNotAcceptMail,
    AuthenticationRequired,
    AuthenticationMechanismTooWeak,
    AuthenticationCredentialsInvalid,
    EncryptionRequiredForAuthentication,
    MailboxUnavailableOrAccessDenied,
    UserNotLocalTryForwardPath,
    ExceededStorageAllocation,
    MailboxNameNotAllowed,
    TransactionFailed,
    MailFromRcptToParametersNotRecognizedOrImplemented,
    AuthenticationMechanismNotAvailable,
}

impl SmtpResponseCode {

    pub fn state(&self) -> CompletionState {
        match self {
            Self::NonStandardSuccess                        |
            Self::SystemStatusReply                         |
            Self::HelpMessage                               |
            Self::ServiceReady                              |
            Self::AuthenticationSuccessful                  |
            Self::RequestedMailActionOkayCompleted          |
            Self::UserNotLocalWillForward                   |
            Self::CannotVerifyUserButWillAttemptDelivery    |
            Self::ServiceClosingTransmissionChannel => CompletionState::PositiveCompletion,
            Self::AuthInputData     |
            Self::StartMailInput    => CompletionState::PositiveIntermediate,
            Self::ServiceNotAvailableClosingTransmissionChannel |
            Self::PasswordTransitionNeeded                      |
            Self::MailboxUnavailable                            |
            Self::LocalErrorInProcessing                        |
            Self::InsufficientSystemStorage                     |
            Self::UnableToAccommodateParameters => CompletionState::TransientNegativeCompletion,
            Self::CommandUnrecognized                                   |
            Self::SyntaxErrorInParameters                               |
            Self::CommandNotImplemented                                 |
            Self::BadSequenceOfCommands                                 |
            Self::CommandParameterNotImplemented                        |
            Self::DomainDoesNotAcceptMail                               |
            Self::AuthenticationRequired                                |
            Self::AuthenticationMechanismTooWeak                        |
            Self::AuthenticationCredentialsInvalid                      |
            Self::EncryptionRequiredForAuthentication                   |
            Self::MailboxUnavailableOrAccessDenied                      |
            Self::UserNotLocalTryForwardPath                            |
            Self::ExceededStorageAllocation                             |
            Self::MailboxNameNotAllowed                                 |
            Self::TransactionFailed                                     |
            Self::MailFromRcptToParametersNotRecognizedOrImplemented    |
            Self::AuthenticationMechanismNotAvailable => CompletionState::PermanentNegativeCompletion,
        }
    }
}

impl fmt::Display for SmtpResponseCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonStandardSuccess                                    => write!(f, "200"),
            Self::SystemStatusReply                                     => write!(f, "211"),
            Self::HelpMessage                                           => write!(f, "214"),
            Self::ServiceReady                                          => write!(f, "220"),
            Self::ServiceClosingTransmissionChannel                     => write!(f, "221"),
            Self::AuthenticationSuccessful                              => write!(f, "235"),
            Self::RequestedMailActionOkayCompleted                      => write!(f, "250"),
            Self::UserNotLocalWillForward                               => write!(f, "251"),
            Self::CannotVerifyUserButWillAttemptDelivery                => write!(f, "252"),
            Self::AuthInputData                                         => write!(f, "334"),
            Self::StartMailInput                                        => write!(f, "354"),
            Self::ServiceNotAvailableClosingTransmissionChannel         => write!(f, "421"),
            Self::PasswordTransitionNeeded                              => write!(f, "432"),
            Self::MailboxUnavailable                                    => write!(f, "450"),
            Self::LocalErrorInProcessing                                => write!(f, "451"),
            Self::InsufficientSystemStorage                             => write!(f, "452"),
            Self::UnableToAccommodateParameters                         => write!(f, "455"),
            Self::CommandUnrecognized                                   => write!(f, "500"),
            Self::SyntaxErrorInParameters                               => write!(f, "501"),
            Self::CommandNotImplemented                                 => write!(f, "502"),
            Self::BadSequenceOfCommands                                 => write!(f, "503"),
            Self::CommandParameterNotImplemented                        => write!(f, "504"),
            Self::DomainDoesNotAcceptMail                               => write!(f, "521"),
            Self::AuthenticationRequired                                => write!(f, "530"),
            Self::AuthenticationMechanismTooWeak                        => write!(f, "534"),
            Self::AuthenticationCredentialsInvalid                      => write!(f, "535"),
            Self::EncryptionRequiredForAuthentication                   => write!(f, "538"),
            Self::MailboxUnavailableOrAccessDenied                      => write!(f, "550"),
            Self::UserNotLocalTryForwardPath                            => write!(f, "551"),
            Self::ExceededStorageAllocation                             => write!(f, "552"),
            Self::MailboxNameNotAllowed                                 => write!(f, "553"),
            Self::TransactionFailed                                     => write!(f, "554"),
            Self::MailFromRcptToParametersNotRecognizedOrImplemented    => write!(f, "555"),
            Self::AuthenticationMechanismNotAvailable                   => write!(f, "556"),
        }
    }
}

impl FromStr for SmtpResponseCode {
    type Err = Error<ErrTag>;

    fn from_str(code: &str) -> Result<Self, Self::Err> {
        Ok(match code {
            "200" => Self::NonStandardSuccess,
            "211" => Self::SystemStatusReply,
            "214" => Self::HelpMessage,
            "220" => Self::ServiceReady,
            "221" => Self::ServiceClosingTransmissionChannel,
            "235" => Self::AuthenticationSuccessful,
            "250" => Self::RequestedMailActionOkayCompleted,
            "251" => Self::UserNotLocalWillForward,
            "252" => Self::CannotVerifyUserButWillAttemptDelivery,
            "334" => Self::AuthInputData,
            "354" => Self::StartMailInput,
            "421" => Self::ServiceNotAvailableClosingTransmissionChannel,
            "432" => Self::PasswordTransitionNeeded,
            "450" => Self::MailboxUnavailable,
            "451" => Self::LocalErrorInProcessing,
            "452" => Self::InsufficientSystemStorage,
            "455" => Self::UnableToAccommodateParameters,
            "500" => Self::CommandUnrecognized,
            "501" => Self::SyntaxErrorInParameters,
            "502" => Self::CommandNotImplemented,
            "503" => Self::BadSequenceOfCommands,
            "504" => Self::CommandParameterNotImplemented,
            "521" => Self::DomainDoesNotAcceptMail,
            "530" => Self::AuthenticationRequired,
            "534" => Self::AuthenticationMechanismTooWeak,
            "535" => Self::AuthenticationCredentialsInvalid,
            "538" => Self::EncryptionRequiredForAuthentication,
            "550" => Self::MailboxUnavailableOrAccessDenied,
            "551" => Self::UserNotLocalTryForwardPath,
            "552" => Self::ExceededStorageAllocation,
            "553" => Self::MailboxNameNotAllowed,
            "554" => Self::TransactionFailed,
            "555" => Self::MailFromRcptToParametersNotRecognizedOrImplemented,
            "556" => Self::AuthenticationMechanismNotAvailable,
            _ => return Err(err!(errmsg!(
                "SMTP code '{}' not recognised.", code,
            ), IO, Network, Wire, Unknown, Input)),
        })
    }
}
