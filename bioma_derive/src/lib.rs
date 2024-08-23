use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput};

#[proc_macro_derive(Message)]
pub fn message_derive(input: TokenStream) -> TokenStream {
    // Parse the input tokens into a syntax tree
    let input = parse_macro_input!(input as DeriveInput);

    // Get the name of the struct or enum
    let name = &input.ident;

    // Generate the implementation
    let expanded = quote! {
        impl Message for #name {}

        #[derive(Clone, ::serde::Serialize, ::serde::Deserialize)]
        #[serde(crate = "::serde")]
        #input
    };

    // Convert back to TokenStream and return
    TokenStream::from(expanded)
}
