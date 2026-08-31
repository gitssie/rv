use gpui::actions;

actions!(
    rv,
    [
        NewConnection,
        ConnectSelected,
        DuplicateSelected,
        DeleteSelected,
        OpenProperties,
        OpenPreferences,
        ToggleViewMode,
        ToggleSidebar,
        FocusSearch,
        QuitApp,
        SessionFullscreen,
        SessionScaleCycle,
        SessionCad,
        SessionDisconnect,
        SessionToggleToolbar,
        SessionMenu,
    ]
);
