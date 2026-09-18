# O texto fixo do livro de músicas impresso — tudo numa página que não é uma música.
#
# Tradução de `en.ftl`, que é onde toda mensagem é escrita primeiro. Uma chave que falta aqui é
# apontada por `every_message_is_translated`; uma chave que só existe aqui é apontada pelo mesmo
# teste, porque uma tradução sem original é uma mensagem que nada pede.
#
# **Todo caractere aqui tem de existir em cp1252.** O livro não embute nenhuma fonte — usa as
# faces base-14 do PDF com WinAnsiEncoding — então qualquer coisa fora desse repertório vira `?` e
# é contada. A contagem só chega a quem gerou o livro pela linha de comando; a rota HTTP não tem
# onde imprimi-la. Veja `en.ftl`.

## Todas as páginas

# Nome próprio, deliberadamente não traduzido.
book-name = KaraokeMachine
book-title = LISTA DE MÚSICAS
# O nome do documento como um arquivo o escreve — veja `en.ftl`.
book-filename = Lista de Músicas

## As quatro colunas

column-artist = ARTISTA
column-code = CÓDIGO
column-title = TÍTULO
column-first-line = INÍCIO DA LETRA

## O que um livro sem linhas diz

book-empty-catalog = Nenhuma música instalada.
book-empty-filter = Nenhuma música corresponde.

## Seções

book-unclassified = Idioma não informado

## A nota sob o título

# O plural do português tem as mesmas duas formas do inglês, mas a palavra muda por inteiro em vez
# de ganhar um `s` — que é exatamente o que a regra escrita à mão em Rust não conseguia dizer.
book-song-count = { $count ->
    [one] { $count } música
   *[other] { $count } músicas
 }
book-of-package = pacote { $package }
book-catalog-version = catálogo { $version }

## Nomes de idiomas, para os títulos de seção

# Parcial de propósito — veja `en.ftl`. Um código sem entrada aqui cai no nome em inglês da tabela
# ISO, que é melhor do que não aparecer.
language-en = Inglês
language-pt = Português
language-es = Espanhol
language-it = Italiano
language-fr = Francês
language-de = Alemão
language-ja = Japonês
language-und = Indeterminado
language-zxx = Sem letra
