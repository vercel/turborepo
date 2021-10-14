module.exports = {
  darkMode: 'class',
  mode: 'jit',
  purge: [
    './components/**/*.tsx',
    './components/safelist.txt',
    './pages/**/*.tsx',
  ],
  theme: {
    extend: {
      colors: {
        dark: 'rgba(17,17,17)',
      },
      screens: {
        sm: '640px',
        md: '768px',
        lg: '1024px',
        betterhover: { raw: '(hover: hover)' },
      },
      typography: (theme) => ({
        DEFAULT: {
          css: {
            a: {
              color: '#3182ce',
              textDecoration: 'none',
              '&:hover': {
                color: '#2c5282',
              },
            },
            img: {
              marginTop: 0,
              marginBottom: 0,
            },
          },
        },
        lg: {
          css: {
            img: {
              marginTop: 0,
              marginBottom: 0,
            },
          },
        },
        dark: {
          css: {
            color: theme('colors.gray.300'),

            h1: {
              color: 'white',
            },
            h2: {
              color: 'white',
            },
            h3: {
              color: 'white',
            },
            h4: {
              color: 'white',
            },
            h5: {
              color: 'white',
            },
            h6: {
              color: 'white',
            },

            strong: {
              color: 'white',
            },

            code: {
              color: 'white',
            },

            figcaption: {
              color: theme('colors.gray.500'),
            },

            '::selection': {
              backgroundColor: '#6f7bb635',
            },
          },
        },
      }),
      opacity: {
        5: '.05',
      },
      spacing: {
        28: '7rem',
      },
      fontSize: {
        '5xl': '2.5rem',
        '6xl': '2.75rem',
        '7xl': '4.5rem',
        '8xl': '6.25rem',
      },
      boxShadow: {
        small: '0 5px 10px rgba(0, 0, 0, 0.12)',
        medium: '0 8px 30px rgba(0, 0, 0, 0.12)',
      },
    },
  },
  variants: {
    typography: ['dark'],
  },
  plugins: [
    require('@tailwindcss/typography'),
    require('@tailwindcss/forms'),
    // ...
  ],
}
