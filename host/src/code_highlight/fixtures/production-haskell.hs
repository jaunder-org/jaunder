{-# LANGUAGE OverloadedStrings, TemplateHaskell #-
module Main where

import Control.Lens
import Data.Maybe
import Data.Text

data Item = Item {
  _itemName :: Text
} deriving (Show)
makeLenses ''Item
makePrisms ''Item

data Container = Container {
  _containerName :: Text,
  _containerItem :: Maybe Item
} deriving (Show)
makeLenses ''Container
makePrisms ''Container

data Collection = Collection {
  _collectionName :: Text,
  _collectionItems :: ![Container]
} deriving (Show)
makeLenses ''Collection
makePrisms ''Collection

collection :: Collectioncollection = Collection "Outermost" [Container "A" (Just (Item "A")),
                                     Container "B" (Nothing),
                                     Container "C" (Just (Item "C")),
                                     Container "D" (Nothing),
                                     Container "E" (Just (Item "E")),
                                     Container "F" (Just (Item "F")),
                                     Container "G" (Just (Item "G"))]

makeDefault :: Collection -> Collection
makeDefault =  over (collectionItems.each.containerItem) (Just . fromMaybe (Item "Bar"))

