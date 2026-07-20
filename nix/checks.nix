{ gossamerPkgs }:

{
  hello-gossamer-app = gossamerPkgs.buildGossamerApplication {
    pname = "hello";
    version = "0.1.0";
    src = ./checks/hello;
  };
}
