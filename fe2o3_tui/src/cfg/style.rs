use crate::{
    cfg::{
        canvas::CanvasLibrary,
        line::LineLibrary,
        scrollbars::ScrollBarsLibrary,
        tab::TabLibrary,
        tbox::TextBoxLibrary,
    },
};

/// A library of pre-calculated drawables.
#[derive(Clone, Debug, Default)]
pub struct StyleLibrary {
    pub canvas:     CanvasLibrary,
    pub line:       LineLibrary,
    pub scrollbars: ScrollBarsLibrary,
    pub tab:        TabLibrary,
    pub tbox:       TextBoxLibrary,
}

impl StyleLibrary {

    // Credits:
    // Image: https://ascii-generator.site/
    pub const DEFAULT_SPLASH: &'static str =
    r#"
    
    
    
    
       ,:************************************************:,   
     -::::**********************************************::::- 
    :***'                                                '***:
    ****    .::::::.  .:::::.                             ****
    ****    `````.:: ,::`````                             ****
    ****     :::::'  :::::::.                             ****
    ****    :::```   :::```:::                            ****
    ****    :::..... :::...:::                            ****
    ****    ::::::::  `:::::'                             ****
    ****                                                  ****
    ****                                                  ****
    ****        ................                          ****
    ****        ::::::::::::::::                          ****
    ****        ::::.-----------                          ****
    ****        ::::.                .---------.          ****
    ****        ::::.              ,:::::::::::::         ****
    ****        :::::..........    ::::.     :::::        ****
    ****        :::::::::::::::    ::::.     :::::        ****
    ****        :::::----------    :::::::::::::::        ****
    ****        ::::.              ::::-----------        ****
    ****        ::::.              ::::.                  ****
    ****        ::::.              :::::::::::::::        ****
    ****        ::::.               ':::::::::::'         ****
    ****                                                  ****
    ****                                                  ****
    ****\                                                /****
     -******************************************************- 
       `**************************************************`   
    
                          I R O N I C
    
          I r o n  I n t e r a c t i v e  C o n s o l e
    
                        Powered by Rust,
                     the Hematite library,
            and the many libraries on which it depends.
    
    
    
    "#;
}
