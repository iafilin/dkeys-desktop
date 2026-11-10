fn main() {
    #[cfg(windows)]
    {
        let mut resource = winresource::WindowsResource::new();
        resource
            .set_icon("assets/icon.ico")
            .set("ProductName", "D-KEYS Desktop")
            .set("FileDescription", "D-KEYS Desktop")
            .set("LegalCopyright", "Игорь Филин");
        resource
            .compile()
            .expect("Не удалось встроить иконку Windows");
    }
}
