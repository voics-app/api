ALTER TABLE users
    ADD COLUMN banner TEXT,
    ADD COLUMN pronouns TEXT CHECK (char_length(pronouns) <= 40),
    ADD COLUMN accent_color INTEGER CHECK (accent_color BETWEEN 0 AND 16777215),
    ADD COLUMN theme_colors INTEGER[] CHECK (
        theme_colors IS NULL OR
        (cardinality(theme_colors) = 2 AND array_ndims(theme_colors) = 1
         AND array_lower(theme_colors, 1) = 1 AND array_position(theme_colors, NULL) IS NULL
         AND 0 <= ALL(theme_colors) AND 16777215 >= ALL(theme_colors))
    );

ALTER TABLE members
    ADD COLUMN banner TEXT,
    ADD COLUMN bio TEXT CHECK (char_length(bio) <= 300);
