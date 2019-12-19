# lardum

A 7DRL [(7-day Roguelike)](http://www.roguebasin.com/index.php?title=7DRL) loosely based on The Sims.

TODO: add screenshot when the 7DRL is up

## Building

For Windows, Make sure you have Visual Studio 2013 or later **with the C++ tools option** installed. You also need the "MSVC ABI" version of the Rust compiler (as opposed to the "GNU ABI" one).

For Linux, make sure you have libsdl2 installed.

```
sudo apt-get install gcc g++ make libsdl2-dev
```

Then, set up the compilation environment, make sure Rust is in your PATH and run Cargo:

```console
$ cd lardum
$ cargo build --release
$ cargo run --release
```

## License

This project is licensed under the GNU General Public License v3.0. See [LICENSE](LICENSE) for more details.
